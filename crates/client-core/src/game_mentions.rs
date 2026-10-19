use model::{GameMention, Id};

pub const MAX_GAMES: usize = 65_536;
pub const MAX_BYTES: usize = 16 * 1024 * 1024;

#[derive(Default, Clone, Copy, PartialEq, Eq)]
pub enum Status {
	#[default]
	Idle,
	Loading,
	Ready,
	Unavailable,
}

#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn game_catalog_rejects_invalid_and_oversized_metadata() {
		let game = GameMention {
			id: Id(42),
			name: "Deadlock".into(),
			icon: None,
		};
		let mut catalog = Catalog::default();
		catalog.finish(Ok(vec![game.clone(), game.clone()]));
		assert_eq!(catalog.games().len(), 1);
		assert_eq!(catalog.label(Id(42)), "Deadlock");
		for invalid in [
			GameMention {
				name: "x".repeat(129),
				..game.clone()
			},
			GameMention {
				icon: Some("../private".into()),
				..game.clone()
			},
			GameMention {
				name: "spoof\u{202e}".into(),
				..game.clone()
			},
		] {
			catalog.finish(Ok(vec![invalid]));
			assert!(catalog.games().is_empty());
			assert!(catalog.status == Status::Unavailable);
		}
		catalog.finish(Ok(vec![game; MAX_GAMES + 1]));
		assert!(catalog.games().is_empty());
	}
}

#[derive(Default)]
pub struct Catalog {
	games: Vec<GameMention>,
	pub status: Status,
	pub requested: bool,
	pub revision: u64,
}

impl Catalog {
	pub fn games(&self) -> &[GameMention] {
		&self.games
	}
	pub fn find(&self, id: Id) -> Option<&GameMention> {
		self.games
			.binary_search_by_key(&id, |game| game.id)
			.ok()
			.map(|i| &self.games[i])
	}
	pub fn request(&mut self) {
		if self.status == Status::Idle {
			self.requested = true;
		}
	}
	pub fn finish(&mut self, result: Result<Vec<GameMention>, &'static str>) {
		self.requested = false;
		self.revision = self.revision.wrapping_add(1);
		self.games = result
			.ok()
			.filter(|games| {
				games.len() <= MAX_GAMES
					&& games.capacity() * size_of::<GameMention>()
						+ games
							.iter()
							.map(|g| {
								g.name.capacity() + g.icon.as_ref().map_or(0, String::capacity)
							})
							.sum::<usize>() <= MAX_BYTES
					&& games.iter().all(|game| {
						game.id.0 != 0
							&& !game.name.is_empty()
							&& game.name.len() <= 128
							&& !game.name.chars().any(|c| {
								c.is_control()
									|| matches!(c, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
							}) && game.icon.as_deref().is_none_or(model::valid_avatar_hash)
					})
			})
			.unwrap_or_default();
		self.games.sort_by_key(|game| game.id);
		self.games.dedup_by_key(|game| game.id);
		self.status = if self.games.is_empty() {
			Status::Unavailable
		} else {
			Status::Ready
		};
	}
	pub fn label(&self, id: Id) -> String {
		self.find(id)
			.map_or_else(|| format!("Unknown game ({id})"), |game| game.name.clone())
	}
}
