//! Read-only catalog details for equipped profile effects and frames.
use crate::{DiscordApi, Failure};
use discord_protocol::collectibles;
use model::{Id, ProfileEffect, ProfileFrame};
use reqwest::Method;
use std::sync::Arc;

/// Catalog products remembered for this session, oldest evicted first.
const CATALOG_ENTRIES: usize = 64;

#[derive(Clone)]
enum Product {
	Effect(Arc<ProfileEffect>),
	Frame(Arc<ProfileFrame>),
	/// The catalog answered, but not with a product this client can show.
	Unsupported,
}

#[derive(Default)]
pub(crate) struct Catalog {
	entries: Vec<(Id, Product)>,
}
impl Catalog {
	fn get(&mut self, sku: Id) -> Option<Product> {
		let index = self.entries.iter().position(|(id, _)| *id == sku)?;
		let entry = self.entries.remove(index);
		let product = entry.1.clone();
		self.entries.push(entry);
		Some(product)
	}
	fn insert(&mut self, sku: Id, product: Product) {
		self.entries.retain(|(id, _)| *id != sku);
		if self.entries.len() >= CATALOG_ENTRIES {
			self.entries.remove(0);
		}
		self.entries.push((sku, product));
	}
}

impl DiscordApi {
	async fn collectible_product(&self, sku: Id, effect: bool) -> Result<Product, Failure> {
		if let Some(product) = self.catalog.lock().ok().and_then(|mut c| c.get(sku)) {
			return Ok(product);
		}
		let bytes = match self
			.request_limited(
				Method::GET,
				&format!("/collectibles-products/{sku}"),
				None,
				collectibles::MAX_PRODUCT_WIRE,
			)
			.await
		{
			Ok(bytes) => bytes,
			// An oversized or refused product will not change this session; remember it so each
			// profile opening does not repeat the read. Other errors may be transient.
			Err(Failure::Capacity | Failure::Forbidden) => {
				self.remember(sku, Product::Unsupported);
				return Ok(Product::Unsupported);
			}
			Err(failure) => return Err(failure),
		};
		let product = if effect {
			collectibles::decode_effect(&bytes, sku).map(|effect| Product::Effect(Arc::new(effect)))
		} else {
			collectibles::decode_frame(&bytes, sku).map(|frame| Product::Frame(Arc::new(frame)))
		}
		.unwrap_or(Product::Unsupported);
		self.remember(sku, product.clone());
		Ok(product)
	}
	fn remember(&self, sku: Id, product: Product) {
		if let Ok(mut catalog) = self.catalog.lock() {
			catalog.insert(sku, product);
		}
	}

	/// Optional artwork for the equipped effect and frame. Only an expired or challenged session
	/// fails the profile; every other failure, including an oversized product, leaves the
	/// profile without that cosmetic.
	pub(super) async fn load_profile_collectibles(
		&self,
		effect: Option<Id>,
		frame: Option<Id>,
	) -> Result<(Option<ProfileEffect>, Option<ProfileFrame>), Failure> {
		let load = |sku: Option<Id>, effect: bool| async move {
			let Some(sku) = sku.filter(|sku| sku.0 != 0) else {
				return Ok(None);
			};
			match self.collectible_product(sku, effect).await {
				Ok(product) => Ok(Some(product)),
				Err(failure) if ends_profile(failure) => Err(failure),
				Err(_) => Ok(None),
			}
		};
		let (effect, frame) = tokio::join!(load(effect, true), load(frame, false));
		let effect = match effect? {
			Some(Product::Effect(effect)) => Some(ProfileEffect::clone(&effect)),
			_ => None,
		};
		let frame = match frame? {
			Some(Product::Frame(frame)) => Some(ProfileFrame::clone(&frame)),
			_ => None,
		};
		Ok((effect, frame))
	}
}

/// Capacity ends a session elsewhere, but here it only describes one optional product.
fn ends_profile(failure: Failure) -> bool {
	failure.ends_session() && !matches!(failure, Failure::Capacity | Failure::CapacityAt(_))
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn catalog_is_bounded_and_refreshes_recent_entries() {
		let mut catalog = Catalog::default();
		for sku in 1..=CATALOG_ENTRIES as u64 {
			catalog.insert(Id(sku), Product::Unsupported);
		}
		assert!(catalog.get(Id(1)).is_some());
		catalog.insert(Id(1000), Product::Unsupported);
		assert_eq!(catalog.entries.len(), CATALOG_ENTRIES);
		assert!(catalog.get(Id(1)).is_some());
		assert!(catalog.get(Id(2)).is_none());
	}

	#[test]
	fn only_a_lost_session_fails_the_profile() {
		assert!(ends_profile(Failure::Expired));
		assert!(ends_profile(Failure::Challenged));
		for optional in [
			Failure::Capacity,
			Failure::CapacityAt("catalog"),
			Failure::Protocol,
			Failure::Network,
			Failure::RateLimited,
		] {
			assert!(!ends_profile(optional), "{optional:?}");
		}
	}
}
