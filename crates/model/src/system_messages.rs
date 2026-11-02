use crate::{Message, User};

/// One styled run of a service-generated message description.
#[derive(Clone, PartialEq, Eq)]
pub struct Segment {
	pub text: String,
	/// Names and values shown in the strong text colour, like Discord's system rows.
	pub strong: bool,
	/// The member this run names, so a client can open their profile.
	pub user: Option<User>,
	/// Localization key replacing `text`, for service wording such as "a member".
	key: Option<&'static str>,
}

/// UI-neutral description of a system message; never inferred from message text.
#[derive(Clone, PartialEq, Eq)]
pub struct SystemMessage {
	/// The English description.
	pub segments: Vec<Segment>,
	/// The original `content` (a new channel or thread name) already appears in `segments`.
	pub content_shown: bool,
	/// Localization key of the sentence. Its placeables are named after `args`.
	pub key: &'static str,
	/// Styled values for the sentence's placeables, such as `$actor` and `$target`.
	pub args: Vec<(&'static str, Segment)>,
}
impl SystemMessage {
	pub fn summary(&self) -> String {
		self.segments.iter().map(|s| s.text.as_str()).collect()
	}

	/// Segments of a translated sentence, or the English ones when a key has no translation.
	/// `translate` formats a key with the given arguments; values arrive as private-use
	/// markers, so names keep their styling and links wherever a language places them.
	pub fn localized(
		&self,
		translate: impl Fn(&str, &[(&'static str, String)]) -> Option<String>,
	) -> Vec<Segment> {
		let markers: Vec<_> = (0..self.args.len())
			.map(|index| (self.args[index].0, marker(index).to_string()))
			.collect();
		let Some(pattern) = translate(self.key, &markers) else {
			return self.segments.clone();
		};
		let args: Vec<_> = self
			.args
			.iter()
			.map(|(name, segment)| {
				let mut segment = segment.clone();
				if let Some(text) = segment.key.and_then(|key| translate(key, &[])) {
					segment.text = text;
				}
				(*name, segment)
			})
			.collect();
		fill(&pattern, &args)
	}
}

const MARKER: u32 = 0xE000;

fn marker(index: usize) -> char {
	char::from_u32(MARKER + index as u32).unwrap_or('\u{FFFD}')
}

/// Splits a pattern at argument markers. Bidi isolation marks around placeables are dropped,
/// since every value becomes its own run.
fn fill(pattern: &str, args: &[(&'static str, Segment)]) -> Vec<Segment> {
	let mut segments = Vec::with_capacity(args.len() * 2 + 1);
	let mut plain = String::new();
	let flush = |plain: &mut String, segments: &mut Vec<Segment>| {
		if !plain.is_empty() {
			segments.push(Segment {
				text: std::mem::take(plain),
				strong: false,
				user: None,
				key: None,
			});
		}
	};
	for c in pattern.chars() {
		let arg = (c as u32)
			.checked_sub(MARKER)
			.and_then(|index| args.get(index as usize));
		if let Some((_, segment)) = arg {
			flush(&mut plain, &mut segments);
			segments.push(segment.clone());
		} else if !matches!(c, '\u{2068}' | '\u{2069}') {
			plain.push(c);
		}
	}
	flush(&mut plain, &mut segments);
	segments
}

/// Message type IDs with a known service description (including unofficial user types).
pub(super) fn is_system(kind: u8) -> bool {
	matches!(
		kind,
		1..=12 | 14..=18 | 21 | 22 | 24..=32 | 36..=39 | 44 | 46 | 55 | 58..=62 | 65 | 67
	)
}

// Public message type IDs: https://docs.discord.com/developers/resources/message#message-types
// Unofficial user types 55, 58..=62, 65, 67: https://docs.discord.food/resources/message#message-types
// Checked 2026-09-12. Descriptions do not grant moderation or calling capabilities.
// Do not infer missing call outcomes, subscription details, or thread contents.
// Keys and English patterns must match `system-message-*` in the UI's en-US catalog.
pub(super) fn describe(message: &Message) -> Option<SystemMessage> {
	if !is_system(message.kind) {
		return None;
	}
	let name = |user: &User| user.name.chars().take(80).collect::<String>();
	let member = |user: &User| Segment {
		text: name(user),
		strong: true,
		user: Some(user.clone()),
		key: None,
	};
	let strong = |text: String| Segment {
		text,
		strong: true,
		user: None,
		key: None,
	};
	let actor = || ("actor", member(&message.author));
	let target = || {
		let segment = message.mentions.first().map_or_else(
			|| Segment {
				text: "a member".into(),
				strong: false,
				user: None,
				key: Some("system-message-a-member"),
			},
			member,
		);
		("target", segment)
	};
	let value = || ("value", strong(message.content.chars().take(100).collect()));
	// The service stores a new channel or thread name in `content`; show it inline like Discord.
	let named = !message.content.trim().is_empty();
	let (key, english, args) = match message.kind {
		1 => (
			"system-message-added",
			"{actor} added {target} to the conversation.",
			vec![actor(), target()],
		),
		2 if message
			.mentions
			.first()
			.is_some_and(|u| u.id == message.author.id) =>
		{
			(
				"system-message-left",
				"{actor} left the conversation.",
				vec![actor()],
			)
		}
		2 => (
			"system-message-removed",
			"{actor} removed {target} from the conversation.",
			vec![actor(), target()],
		),
		3 => (
			"system-message-call",
			"{actor} started a call.",
			vec![actor()],
		),
		4 if named => (
			"system-message-channel-name-value",
			"{actor} changed the channel name: {value}",
			vec![actor(), value()],
		),
		4 => (
			"system-message-channel-name",
			"{actor} changed the channel name.",
			vec![actor()],
		),
		5 => (
			"system-message-channel-icon",
			"{actor} changed the channel icon.",
			vec![actor()],
		),
		6 => (
			"system-message-pinned",
			"{actor} pinned a message to this channel.",
			vec![actor()],
		),
		7 => (
			"system-message-joined",
			"Welcome, {actor}! Joined the server.",
			vec![actor()],
		),
		8 => (
			"system-message-boosted",
			"{actor} boosted the server!",
			vec![actor()],
		),
		9..=11 => (
			"system-message-boosted-level",
			"{actor} boosted the server! The server reached level {level}.",
			vec![
				actor(),
				(
					"level",
					Segment {
						text: (message.kind - 8).to_string(),
						strong: false,
						user: None,
						key: None,
					},
				),
			],
		),
		12 => (
			"system-message-channel-follow",
			"{actor} added a channel follow.",
			vec![actor()],
		),
		14 => (
			"system-message-discovery-disqualified",
			"This server is no longer eligible for Server Discovery.",
			vec![],
		),
		15 => (
			"system-message-discovery-requalified",
			"This server is eligible for Server Discovery again.",
			vec![],
		),
		16 => (
			"system-message-discovery-warning",
			"Server Discovery eligibility warning.",
			vec![],
		),
		17 => (
			"system-message-discovery-final-warning",
			"Final Server Discovery eligibility warning.",
			vec![],
		),
		18 if named => (
			"system-message-thread-value",
			"{actor} started a thread: {value}",
			vec![actor(), value()],
		),
		18 => (
			"system-message-thread",
			"{actor} started a thread.",
			vec![actor()],
		),
		21 => (
			"system-message-thread-starter",
			"Thread started from an earlier message.",
			vec![],
		),
		22 => (
			"system-message-invite-reminder",
			"Invite reminder · Invite people to this server.",
			vec![],
		),
		24 => ("system-message-automod", "AutoMod action.", vec![]),
		25 => (
			"system-message-role-subscription",
			"{actor} purchased or renewed a role subscription.",
			vec![actor()],
		),
		26 => (
			"system-message-premium-upsell",
			"Premium subscription offer.",
			vec![],
		),
		27 => (
			"system-message-stage-start",
			"{actor} started a Stage.",
			vec![actor()],
		),
		28 => ("system-message-stage-end", "The Stage ended.", vec![]),
		29 => (
			"system-message-stage-speaker",
			"{actor} became a Stage speaker.",
			vec![actor()],
		),
		30 => (
			"system-message-stage-raise-hand",
			"{actor} requested to speak.",
			vec![actor()],
		),
		31 => (
			"system-message-stage-topic",
			"{actor} changed the Stage topic.",
			vec![actor()],
		),
		32 => (
			"system-message-application-subscription",
			"Server application premium subscription.",
			vec![],
		),
		36 => (
			"system-message-raid-alert-enabled",
			"Server alert mode enabled.",
			vec![],
		),
		37 => (
			"system-message-raid-alert-disabled",
			"Server alert mode disabled.",
			vec![],
		),
		38 => (
			"system-message-raid-report",
			"A server raid was reported.",
			vec![],
		),
		39 => (
			"system-message-false-alarm",
			"A server incident was reported as a false alarm.",
			vec![],
		),
		44 => ("system-message-purchase", "Purchase notification.", vec![]),
		46 => match poll_question(message) {
			Some(question) => (
				"system-message-poll-closed",
				"{actor}'s poll {question} has closed.",
				vec![actor(), ("question", strong(question))],
			),
			None => ("system-message-poll-result", "Poll results.", vec![]),
		},
		55 => (
			"system-message-stream-hd",
			"{actor} upgraded the stream to HD.",
			vec![actor()],
		),
		58 => (
			"system-message-report-deleted",
			"{actor} deleted a reported message.",
			vec![actor()],
		),
		59 => (
			"system-message-timed-out",
			"{actor} timed out {target}.",
			vec![actor(), target()],
		),
		60 => (
			"system-message-kicked",
			"{actor} kicked {target}.",
			vec![actor(), target()],
		),
		61 => (
			"system-message-banned",
			"{actor} banned {target}.",
			vec![actor(), target()],
		),
		62 => (
			"system-message-report-resolved",
			"{actor} resolved a report.",
			vec![actor()],
		),
		65 => (
			"system-message-voice-hangout",
			"{actor} started a voice hangout.",
			vec![actor()],
		),
		67 => (
			"system-message-friend-accepted",
			"{actor} accepted your friend request.",
			vec![actor()],
		),
		_ => return None,
	};
	let mut pattern = english.to_owned();
	for (index, (name, _)) in args.iter().enumerate() {
		pattern = pattern.replace(&format!("{{{name}}}"), &marker(index).to_string());
	}
	Some(SystemMessage {
		segments: fill(&pattern, &args),
		content_shown: key.ends_with("-value"),
		key,
		args,
	})
}

/// The closed poll's question from the `poll_result` embed, shortened like Discord's row.
fn poll_question(message: &Message) -> Option<String> {
	let question = message
		.embeds
		.iter()
		.find(|embed| embed.kind == "poll_result")?
		.field("poll_question_text")?
		.trim();
	if question.is_empty() {
		return None;
	}
	let mut chars = question.chars();
	let mut text: String = chars.by_ref().take(48).collect();
	if chars.next().is_some() {
		text.push('…');
	}
	Some(text)
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::Id;

	fn message(kind: u8) -> Message {
		Message {
			poll: None,
			flags: 0,
			sticker_items: vec![],
			components: vec![],
			application_id: None,
			ephemeral: false,
			kind,
			author: User {
				primary_guild: None,
				cosmetics: None,
				id: Id(3),
				name: "Robin".into(),
				avatar: None,
				webhook: false,
				kind: Default::default(),
				discriminator: 0,
			},
			mentions: vec![User {
				primary_guild: None,
				cosmetics: None,
				id: Id(4),
				name: "Casey".into(),
				avatar: None,
				webhook: false,
				kind: Default::default(),
				discriminator: 0,
			}],
			content: "general".into(),
			reactions: None,
			id: Id(1),
			channel: Id(2),
			author_nick: None,
			author_roles: vec![],
			mention_roles: vec![],
			mention_everyone: false,
			suppress_notifications: false,
			edited: false,
			edited_at: None,
			revision: 0,
			nonce: None,
			reply_to: None,
			reply_deleted: false,
			interaction: None,
			forwarded: false,
			unsupported: false,
			extra_content: Default::default(),
			embeds: vec![],
			embeds_suppressed: false,
			attachments: vec![],
		}
	}

	#[test]
	fn every_known_kind_describes_and_unknown_kinds_do_not() {
		for kind in 0..=255u8 {
			let message = message(kind);
			assert_eq!(
				message.system_message().is_some(),
				is_system(kind),
				"kind {kind}"
			);
			assert_eq!(message.system_summary().is_some(), is_system(kind));
			if let Some(system) = message.system_message() {
				assert!(!system.segments.is_empty());
				assert!(system.segments.iter().all(|s| !s.text.is_empty()));
				assert!(system.summary().chars().count() < 200);
			}
		}
	}
}
