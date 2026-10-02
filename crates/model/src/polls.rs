//! Bounded poll content and explicit user-authored creation requests.
use crate::ReactionEmoji;
use serde::{Deserialize, Serialize};

pub const MAX_ANSWERS: usize = 10;
pub const MAX_BYTES: usize = 8 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Media {
	#[serde(default)]
	pub text: String,
	pub emoji: Option<ReactionEmoji>,
}
impl Media {
	pub fn valid(&self) -> bool {
		self.text.chars().count() <= 55
			&& self.text.len() <= 220
			&& (!self.text.trim().is_empty() || self.emoji.is_some())
			&& self
				.emoji
				.as_ref()
				.is_none_or(|e| e.valid() && e.id.is_none_or(|id| id.0 != 0))
	}
	fn bytes(&self) -> usize {
		self.text.capacity()
			+ self
				.emoji
				.as_ref()
				.and_then(|e| e.name.as_ref())
				.map_or(0, String::capacity)
	}
}
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Answer {
	pub id: u32,
	pub media: Media,
	pub votes: u32,
	pub me: bool,
}
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Poll {
	pub question: String,
	pub answers: Vec<Answer>,
	pub expiry: Option<i128>,
	pub multiselect: bool,
	pub results_known: bool,
	pub finalized: bool,
}
impl Poll {
	pub fn bytes(&self) -> usize {
		std::mem::size_of::<Self>()
			+ self.question.capacity()
			+ self.answers.capacity() * std::mem::size_of::<Answer>()
			+ self.answers.iter().map(|a| a.media.bytes()).sum::<usize>()
	}
	pub fn valid(&self) -> bool {
		valid_question(&self.question)
			&& (2..=MAX_ANSWERS).contains(&self.answers.len())
			&& self.bytes() <= MAX_BYTES
			&& self.answers.iter().enumerate().all(|(i, a)| {
				a.id != 0
					&& a.media.valid()
					&& !self.answers[..i].iter().any(|other| other.id == a.id)
			})
	}
	pub fn retain_results(&mut self, previous: &Self) {
		if !self.results_known
			&& previous.results_known
			&& self.question == previous.question
			&& self.answers.len() == previous.answers.len()
			&& self
				.answers
				.iter()
				.zip(&previous.answers)
				.all(|(a, b)| a.id == b.id && a.media == b.media)
		{
			for (answer, old) in self.answers.iter_mut().zip(&previous.answers) {
				answer.votes = old.votes;
				answer.me = old.me;
			}
			self.results_known = true;
			self.finalized |= previous.finalized;
		}
	}
	pub fn ended(&self, now_ms: i128) -> bool {
		self.finalized || self.expiry.is_some_and(|expiry| expiry <= now_ms)
	}
	pub fn votes(&self) -> u64 {
		self.answers.iter().map(|a| u64::from(a.votes)).sum()
	}
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Create {
	pub question: String,
	pub answers: Vec<Media>,
	pub duration: u16,
	pub multiselect: bool,
}
impl Create {
	pub fn bytes(&self) -> usize {
		std::mem::size_of::<Self>()
			+ self.question.capacity()
			+ self.answers.capacity() * std::mem::size_of::<Media>()
			+ self.answers.iter().map(Media::bytes).sum::<usize>()
	}
	pub fn valid(&self) -> bool {
		valid_question(&self.question)
			&& (2..=MAX_ANSWERS).contains(&self.answers.len())
			&& (1..=768).contains(&self.duration)
			&& self.answers.iter().all(Media::valid)
			&& self.bytes() <= MAX_BYTES
	}
	pub fn fixture(&self, now_ms: i128) -> Poll {
		Poll {
			question: self.question.clone(),
			answers: self
				.answers
				.iter()
				.enumerate()
				.map(|(i, media)| Answer {
					id: i as u32 + 1,
					media: media.clone(),
					votes: 0,
					me: false,
				})
				.collect(),
			expiry: Some(now_ms + i128::from(self.duration) * 3_600_000),
			multiselect: self.multiselect,
			results_known: true,
			finalized: false,
		}
	}
}
fn valid_question(text: &str) -> bool {
	!text.trim().is_empty() && text.chars().count() <= 300 && text.len() <= 1200
}
