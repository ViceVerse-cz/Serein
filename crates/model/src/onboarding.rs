//! Server rules screening (member verification) and onboarding questions for the current user.
use crate::Id;

/// Guild member flag set once the member finished onboarding.
pub const COMPLETED_ONBOARDING: u64 = 1 << 1;
pub const MAX_FIELDS: usize = 5;
pub const MAX_RULES: usize = 16;
pub const MAX_CHOICES: usize = 8;
pub const MAX_PROMPTS: usize = 50;
pub const MAX_OPTIONS: usize = 50;
pub const MAX_TEXT: usize = 1000;
pub const MAX_SHORT_TEXT: usize = 150;

/// Server features that can hold a fresh member back from talking.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Features {
	pub verification: bool,
	pub onboarding: bool,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MemberGate {
	pub pending: bool,
	pub flags: u64,
}
/// One gateway observation; `None` parts are unknown in this event and keep their old value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GuildGate {
	pub guild: Id,
	pub features: Option<Features>,
	pub member: Option<MemberGate>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FieldKind {
	Terms,
	TextInput,
	Paragraph,
	MultipleChoice,
	Unknown,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Field {
	pub kind: FieldKind,
	/// The wire `field_type`, echoed back unchanged in the join request.
	pub wire_kind: String,
	pub label: String,
	pub description: Option<String>,
	pub placeholder: Option<String>,
	pub rules: Vec<String>,
	pub choices: Vec<String>,
	pub required: bool,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Verification {
	pub version: Option<String>,
	pub description: Option<String>,
	pub fields: Vec<Field>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PromptOption {
	pub id: Id,
	pub title: String,
	pub description: Option<String>,
	/// Unicode emoji or a custom emoji name.
	pub emoji: Option<String>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Prompt {
	pub id: Id,
	pub title: String,
	pub single_select: bool,
	pub required: bool,
	pub options: Vec<PromptOption>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Onboarding {
	/// Only prompts shown during onboarding; customization-only prompts are dropped.
	pub prompts: Vec<Prompt>,
	/// Option IDs the member already chose.
	pub responses: Vec<Id>,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Form {
	pub verification: Option<Verification>,
	pub onboarding: Option<Onboarding>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Answer {
	Terms(bool),
	Text(String),
	Choice(Option<usize>),
}
/// Everything the member chose; answers are index-aligned with the verification fields.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Submission {
	pub onboarding: Option<Vec<Id>>,
	pub verification: Option<Vec<Answer>>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
	/// The member can now talk.
	Approved,
	/// A moderator must review the application first.
	Submitted,
	Rejected,
}

fn text_bytes(text: &str) -> usize {
	text.len()
}
fn opt_bytes(text: &Option<String>) -> usize {
	text.as_ref().map_or(0, |t| text_bytes(t))
}
impl Field {
	pub fn answered(&self, answer: &Answer) -> bool {
		match (&self.kind, answer) {
			(FieldKind::Terms, Answer::Terms(agreed)) => *agreed,
			(FieldKind::TextInput | FieldKind::Paragraph, Answer::Text(text)) => {
				!text.trim().is_empty()
			}
			(FieldKind::MultipleChoice, Answer::Choice(choice)) => {
				choice.is_some_and(|c| c < self.choices.len())
			}
			_ => false,
		}
	}
	pub fn max_chars(&self) -> usize {
		if self.kind == FieldKind::Paragraph {
			MAX_TEXT
		} else {
			MAX_SHORT_TEXT
		}
	}
}
impl Verification {
	/// A form Serein cannot answer completely (an unknown required question).
	pub fn supported(&self) -> bool {
		self.fields
			.iter()
			.all(|f| !f.required || f.kind != FieldKind::Unknown)
	}
	/// Non-rules questions need a moderator to approve the application.
	pub fn needs_review(&self) -> bool {
		self.fields.iter().any(|f| f.kind != FieldKind::Terms)
	}
	pub fn complete(&self, answers: &[Answer]) -> bool {
		answers.len() == self.fields.len()
			&& self.supported()
			&& self.fields.iter().zip(answers).all(|(field, answer)| {
				(!field.required || field.answered(answer))
					&& match answer {
						Answer::Text(text) => text.chars().count() <= field.max_chars(),
						_ => true,
					}
			})
	}
}
impl Onboarding {
	pub fn complete(&self, chosen: &[Id]) -> bool {
		self.prompts.iter().all(|prompt| {
			let picked = prompt
				.options
				.iter()
				.filter(|option| chosen.contains(&option.id))
				.count();
			(!prompt.required || picked > 0) && (!prompt.single_select || picked <= 1)
		}) && chosen.iter().all(|id| {
			self.prompts
				.iter()
				.any(|prompt| prompt.options.iter().any(|option| option.id == *id))
		})
	}
}
impl Form {
	pub fn is_empty(&self) -> bool {
		self.verification.is_none() && self.onboarding.is_none()
	}
	pub fn valid(&self) -> bool {
		self.verification.as_ref().is_none_or(|v| {
			v.fields.len() <= MAX_FIELDS
				&& v.fields
					.iter()
					.all(|f| f.rules.len() <= MAX_RULES && f.choices.len() <= MAX_CHOICES)
		}) && self.onboarding.as_ref().is_none_or(|o| {
			o.prompts.len() <= MAX_PROMPTS
				&& o.prompts.iter().all(|p| p.options.len() <= MAX_OPTIONS)
		})
	}
	pub fn bytes(&self) -> usize {
		size_of::<Self>()
			+ self.verification.as_ref().map_or(0, |v| {
				size_of::<Verification>()
					+ opt_bytes(&v.version)
					+ opt_bytes(&v.description)
					+ v.fields
						.iter()
						.map(|f| {
							size_of::<Field>()
								+ f.wire_kind.len() + f.label.len()
								+ opt_bytes(&f.description)
								+ opt_bytes(&f.placeholder)
								+ f.rules
									.iter()
									.chain(&f.choices)
									.map(|t| size_of::<String>() + t.len())
									.sum::<usize>()
						})
						.sum::<usize>()
			}) + self.onboarding.as_ref().map_or(0, |o| {
			size_of::<Onboarding>()
				+ o.responses.len() * size_of::<Id>()
				+ o.prompts
					.iter()
					.map(|p| {
						size_of::<Prompt>()
							+ p.title.len() + p
							.options
							.iter()
							.map(|option| {
								size_of::<PromptOption>()
									+ option.title.len() + opt_bytes(&option.description)
									+ opt_bytes(&option.emoji)
							})
							.sum::<usize>()
					})
					.sum::<usize>()
		})
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn field(kind: FieldKind, required: bool) -> Field {
		Field {
			kind,
			wire_kind: String::new(),
			label: "Rules".into(),
			description: None,
			placeholder: None,
			rules: vec!["Be kind".into()],
			choices: vec!["A".into(), "B".into()],
			required,
		}
	}

	#[test]
	fn verification_requires_every_required_answer_and_known_kinds() {
		let form = Verification {
			version: None,
			description: None,
			fields: vec![
				field(FieldKind::Terms, true),
				field(FieldKind::MultipleChoice, true),
				field(FieldKind::TextInput, false),
			],
		};
		let mut answers = vec![
			Answer::Terms(true),
			Answer::Choice(Some(1)),
			Answer::Text(String::new()),
		];
		assert!(form.complete(&answers));
		answers[1] = Answer::Choice(Some(2));
		assert!(!form.complete(&answers));
		answers[1] = Answer::Choice(Some(0));
		answers[2] = Answer::Text("x".repeat(MAX_SHORT_TEXT + 1));
		assert!(!form.complete(&answers));
		let unknown = Verification {
			fields: vec![field(FieldKind::Unknown, true)],
			..form
		};
		assert!(!unknown.supported());
	}

	#[test]
	fn onboarding_respects_required_and_single_select_prompts() {
		let option = |id| PromptOption {
			id: Id(id),
			title: "Option".into(),
			description: None,
			emoji: None,
		};
		let onboarding = Onboarding {
			prompts: vec![Prompt {
				id: Id(1),
				title: "Pick".into(),
				single_select: true,
				required: true,
				options: vec![option(10), option(11)],
			}],
			responses: Vec::new(),
		};
		assert!(!onboarding.complete(&[]));
		assert!(onboarding.complete(&[Id(10)]));
		assert!(!onboarding.complete(&[Id(10), Id(11)]));
		assert!(!onboarding.complete(&[Id(99)]));
	}
}
