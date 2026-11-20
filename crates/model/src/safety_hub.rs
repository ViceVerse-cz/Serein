//! Read-only account standing from Discord's Safety Hub. Unofficial normal-client behavior;
//! see `docs/discord-compatibility.md`. Never persisted and never inferred from flags, DMs or
//! profile data. Flagged message bodies and attachments are deliberately not retained.
use crate::Id;

/// Retained classifications. Extra wire rows are dropped before they enter memory.
pub const MAX_CLASSIFICATIONS: usize = 16;
pub const MAX_ACTIONS: usize = 8;
pub const MAX_ACTION_DESCRIPTIONS: usize = 4;
pub const MAX_APPEAL_ELIGIBILITY: usize = 3;
pub const MAX_TEXT: usize = 512;
pub const MAX_LINK: usize = 2048;
/// Memory retained beside one explicit Safety Hub read.
pub const MAX_BYTES: usize = 32 * 1024;

/// Documented `account_standing.state` values. Unknown or missing values stay unavailable.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AccountStanding {
	AllGood,
	Limited,
	VeryLimited,
	AtRisk,
	Suspended,
}
impl AccountStanding {
	/// Only the documented states are typed; never default an unknown value to `AllGood`.
	pub fn parse(value: u32) -> Option<Self> {
		Some(match value {
			100 => Self::AllGood,
			200 => Self::Limited,
			300 => Self::VeryLimited,
			400 => Self::AtRisk,
			500 => Self::Suspended,
			_ => return None,
		})
	}
}

/// Documented `appeal_eligibility` values; unknown values are dropped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AppealEligibility {
	Dsa,
	InApp,
	AgeVerification,
}
impl AppealEligibility {
	pub fn parse(value: u32) -> Option<Self> {
		Some(match value {
			1 => Self::Dsa,
			2 => Self::InApp,
			3 => Self::AgeVerification,
			_ => return None,
		})
	}
}

/// Documented classification appeal status; an unknown status is unavailable.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AppealStatus {
	ReviewPending,
	Upheld,
	Invalidated,
}
impl AppealStatus {
	pub fn parse(value: u32) -> Option<Self> {
		Some(match value {
			1 => Self::ReviewPending,
			2 => Self::Upheld,
			3 => Self::Invalidated,
			_ => return None,
		})
	}
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Action {
	pub id: Id,
	/// Raw documented action type; unknown values display their descriptions or a generic label.
	pub action_type: u32,
	pub descriptions: Vec<String>,
}
impl Action {
	pub fn valid(&self) -> bool {
		self.id.0 != 0
			&& self.descriptions.len() <= MAX_ACTION_DESCRIPTIONS
			&& self.descriptions.capacity() <= MAX_ACTION_DESCRIPTIONS
			&& self
				.descriptions
				.iter()
				.all(|text| text.chars().count() <= MAX_TEXT)
	}
	fn heap_bytes(&self) -> usize {
		self.descriptions
			.iter()
			.map(|text| text.capacity())
			.sum::<usize>()
			+ self.descriptions.capacity() * std::mem::size_of::<String>()
	}
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Classification {
	pub id: Id,
	/// Raw documented classification type; shown generically when unknown.
	pub classification_type: u32,
	pub description: String,
	pub explainer_link: Option<String>,
	pub actions: Vec<Action>,
	/// Unix seconds when the classification expires, when the service supplied a valid time.
	pub max_expiration_time: Option<i64>,
	pub appeal_status: Option<AppealStatus>,
}
impl Classification {
	pub fn valid(&self) -> bool {
		self.id.0 != 0
			&& self.description.chars().count() <= MAX_TEXT
			&& self
				.explainer_link
				.as_ref()
				.is_none_or(|link| link.len() <= MAX_LINK)
			&& self.actions.len() <= MAX_ACTIONS
			&& self.actions.capacity() <= MAX_ACTIONS
			&& self.actions.iter().all(Action::valid)
	}
	fn heap_bytes(&self) -> usize {
		self.description.capacity()
			+ self.explainer_link.as_ref().map_or(0, String::capacity)
			+ self.actions.capacity() * std::mem::size_of::<Action>()
			+ self.actions.iter().map(Action::heap_bytes).sum::<usize>()
	}
}

/// The explicitly requested account Safety Hub. Unknown standing is `None`, never `AllGood`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Snapshot {
	pub standing: Option<AccountStanding>,
	pub classifications: Vec<Classification>,
	pub is_appeal_eligible: bool,
	pub is_dsa_eligible: bool,
	pub appeal_eligibility: Vec<AppealEligibility>,
}
impl Snapshot {
	pub fn valid(&self) -> bool {
		self.classifications.len() <= MAX_CLASSIFICATIONS
			&& self.classifications.capacity() <= MAX_CLASSIFICATIONS
			&& self.classifications.iter().all(Classification::valid)
			&& self.appeal_eligibility.len() <= MAX_APPEAL_ELIGIBILITY
			&& self.appeal_eligibility.capacity() <= MAX_APPEAL_ELIGIBILITY
			&& self
				.appeal_eligibility
				.iter()
				.enumerate()
				.all(|(index, value)| !self.appeal_eligibility[..index].contains(value))
			&& self.bytes() <= MAX_BYTES
	}
	pub fn bytes(&self) -> usize {
		std::mem::size_of::<Self>()
			+ self.classifications.capacity() * std::mem::size_of::<Classification>()
			+ self
				.classifications
				.iter()
				.map(Classification::heap_bytes)
				.sum::<usize>()
			+ self.appeal_eligibility.capacity() * std::mem::size_of::<AppealEligibility>()
	}
}
