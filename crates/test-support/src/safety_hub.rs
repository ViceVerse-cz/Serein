//! Handcrafted Safety Hub fixtures for the offline preview and focused UI/reducer tests.
//! No network import; never evidence of live compatibility.
use model::{
	Id,
	safety_hub::{
		AccountStanding, Action, AppealEligibility, AppealStatus, Classification, Snapshot,
	},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fixture {
	Good,
	Limited,
	Unavailable,
	Violation,
}
impl Fixture {
	pub const ALL: [Self; 4] = [
		Self::Good,
		Self::Limited,
		Self::Unavailable,
		Self::Violation,
	];

	pub fn parse(value: &str) -> Option<Self> {
		Some(match value.trim().to_ascii_lowercase().as_str() {
			"good" | "all-good" | "all_good" => Self::Good,
			"limited" => Self::Limited,
			"unavailable" => Self::Unavailable,
			"violation" | "with-violation" | "with_violation" => Self::Violation,
			_ => return None,
		})
	}

	pub fn snapshot(self) -> Snapshot {
		match self {
			Self::Good => Snapshot {
				standing: Some(AccountStanding::AllGood),
				..Default::default()
			},
			Self::Limited => Snapshot {
				standing: Some(AccountStanding::Limited),
				is_appeal_eligible: true,
				appeal_eligibility: vec![AppealEligibility::InApp],
				..Default::default()
			},
			// The service omitted, malformed or returned an unknown standing state.
			Self::Unavailable => Snapshot {
				standing: None,
				..Default::default()
			},
			Self::Violation => Snapshot {
				standing: Some(AccountStanding::AtRisk),
				classifications: vec![Classification {
					id: Id(1411751868690075770),
					classification_type: 3030,
					description: "Synthetic spam classification for the offline preview.".into(),
					explainer_link: Some(
						"https://support.discord.com/hc/articles/18210965981847".into(),
					),
					actions: vec![Action {
						id: Id(1411751868690075771),
						action_type: 4,
						descriptions: vec!["Synthetic warning for the offline preview.".into()],
					}],
					max_expiration_time: Some(1_788_000_000),
					appeal_status: Some(AppealStatus::ReviewPending),
				}],
				is_appeal_eligible: true,
				is_dsa_eligible: false,
				appeal_eligibility: vec![AppealEligibility::InApp],
			},
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn every_fixture_is_bounded_and_unavailable_keeps_unknown_standing() {
		for fixture in Fixture::ALL {
			let snapshot = fixture.snapshot();
			assert!(snapshot.valid(), "{fixture:?}");
			assert!(snapshot.bytes() <= model::safety_hub::MAX_BYTES);
		}
		assert_eq!(Fixture::Unavailable.snapshot().standing, None);
		assert_eq!(
			Fixture::Violation.snapshot().classifications[0].actions[0]
				.descriptions
				.len(),
			1
		);
		assert_eq!(Fixture::parse("with-violation"), Some(Fixture::Violation));
		assert_eq!(Fixture::parse("unknown"), None);
	}
}
