//! Polls retain a presence marker for unsupported future layouts.
use model::polls::{Answer, Media, Poll};
use serde::{Deserialize, Deserializer};

pub struct PollDto(pub Option<Box<Poll>>);
#[derive(Deserialize)]
struct Wire {
	question: Text,
	answers: Vec<WireAnswer>,
	expiry: Option<crate::Timestamp>,
	allow_multiselect: bool,
	layout_type: u8,
	results: Option<Results>,
}
#[derive(Deserialize)]
struct Text {
	text: String,
}
#[derive(Deserialize)]
struct WireAnswer {
	answer_id: u32,
	poll_media: Media,
}
#[derive(Deserialize)]
struct Results {
	is_finalized: bool,
	answer_counts: Vec<Count>,
}
#[derive(Deserialize)]
struct Count {
	id: u32,
	count: u32,
	me_voted: bool,
}
impl<'de> Deserialize<'de> for PollDto {
	fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
		let raw = Box::<serde_json::value::RawValue>::deserialize(d)?;
		if raw.get().len() > 128 * 1024 {
			return Err(serde::de::Error::custom("Poll exceeds capacity"));
		}
		serde_json::from_str::<crate::extra_content::Object>(raw.get())
			.map_err(serde::de::Error::custom)?;
		let poll = serde_json::from_str::<Wire>(raw.get())
			.ok()
			.and_then(|wire| {
				if wire.layout_type != 1 || wire.answers.len() > model::polls::MAX_ANSWERS {
					return None;
				}
				if wire.results.as_ref().is_some_and(|r| {
					r.answer_counts.len() > model::polls::MAX_ANSWERS
						|| r.answer_counts.iter().enumerate().any(|(i, count)| {
							!wire.answers.iter().any(|a| a.answer_id == count.id)
								|| r.answer_counts[..i]
									.iter()
									.any(|other| other.id == count.id)
						})
				}) {
					return None;
				}
				let poll = Poll {
					question: wire.question.text,
					answers: wire
						.answers
						.into_iter()
						.map(|a| {
							let count = wire
								.results
								.as_ref()
								.and_then(|r| r.answer_counts.iter().find(|c| c.id == a.answer_id));
							Answer {
								id: a.answer_id,
								media: a.poll_media,
								votes: count.map_or(0, |c| c.count),
								me: count.is_some_and(|c| c.me_voted),
							}
						})
						.collect(),
					expiry: wire.expiry.map(|t| t.0 / 1_000_000),
					multiselect: wire.allow_multiselect,
					results_known: wire.results.is_some(),
					finalized: wire.results.is_some_and(|r| r.is_finalized),
				};
				poll.valid().then(|| Box::new(poll))
			});
		Ok(Self(poll))
	}
}

#[derive(Deserialize)]
pub struct Vote {
	pub channel_id: model::Id,
	pub message_id: model::Id,
	pub user_id: model::Id,
	pub answer_id: u32,
}
