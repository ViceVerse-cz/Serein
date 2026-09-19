use crate::{Message, User};

/// One styled run of a service-generated message description.
#[derive(Clone, PartialEq, Eq)]
pub struct Segment {
	pub text: String,
	/// Names and values shown in the strong text colour, like Discord's system rows.
	pub strong: bool,
	/// The member this run names, so a client can open their profile.
	pub user: Option<User>,
}

/// UI-neutral description of a system message; never inferred from message text.
#[derive(Clone, PartialEq, Eq)]
pub struct SystemMessage {
	pub segments: Vec<Segment>,
	/// The original `content` (a new channel or thread name) already appears in `segments`.
	pub content_shown: bool,
}
impl SystemMessage {
	pub fn summary(&self) -> String {
		self.segments.iter().map(|s| s.text.as_str()).collect()
	}
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
pub(super) fn describe(message: &Message) -> Option<SystemMessage> {
	if !is_system(message.kind) {
		return None;
	}
	let name = |user: &User| user.name.chars().take(80).collect::<String>();
	let plain = |text: &str| Segment {
		text: text.into(),
		strong: false,
		user: None,
	};
	let member = |user: &User| Segment {
		text: name(user),
		strong: true,
		user: Some(user.clone()),
	};
	let actor = || member(&message.author);
	let target = || {
		message
			.mentions
			.first()
			.map_or_else(|| plain("a member"), member)
	};
	let value = || Segment {
		text: message.content.chars().take(100).collect(),
		strong: true,
		user: None,
	};
	let mut content_shown = false;
	let mut named = |verb: &str| {
		// The service stores the new name in `content`; show it inline like Discord.
		if message.content.trim().is_empty() {
			vec![actor(), plain(verb), plain(".")]
		} else {
			content_shown = true;
			vec![actor(), plain(verb), plain(": "), value()]
		}
	};
	let segments = match message.kind {
		1 => vec![
			actor(),
			plain(" added "),
			target(),
			plain(" to the conversation."),
		],
		2 if message
			.mentions
			.first()
			.is_some_and(|u| u.id == message.author.id) =>
		{
			vec![actor(), plain(" left the conversation.")]
		}
		2 => vec![
			actor(),
			plain(" removed "),
			target(),
			plain(" from the conversation."),
		],
		3 => vec![actor(), plain(" started a call.")],
		4 => named(" changed the channel name"),
		5 => vec![actor(), plain(" changed the channel icon.")],
		6 => vec![actor(), plain(" pinned a message to this channel.")],
		7 => vec![plain("Welcome, "), actor(), plain("! Joined the server.")],
		8 => vec![actor(), plain(" boosted the server!")],
		9..=11 => vec![
			actor(),
			plain(&format!(
				" boosted the server! The server reached level {}.",
				message.kind - 8
			)),
		],
		12 => vec![actor(), plain(" added a channel follow.")],
		14 => vec![plain(
			"This server is no longer eligible for Server Discovery.",
		)],
		15 => vec![plain("This server is eligible for Server Discovery again.")],
		16 => vec![plain("Server Discovery eligibility warning.")],
		17 => vec![plain("Final Server Discovery eligibility warning.")],
		18 => named(" started a thread"),
		21 => vec![plain("Thread started from an earlier message.")],
		22 => vec![plain("Invite reminder · Invite people to this server.")],
		24 => vec![plain("AutoMod action.")],
		25 => vec![actor(), plain(" purchased or renewed a role subscription.")],
		26 => vec![plain("Premium subscription offer.")],
		27 => vec![actor(), plain(" started a Stage.")],
		28 => vec![plain("The Stage ended.")],
		29 => vec![actor(), plain(" became a Stage speaker.")],
		30 => vec![actor(), plain(" requested to speak.")],
		31 => vec![actor(), plain(" changed the Stage topic.")],
		32 => vec![plain("Server application premium subscription.")],
		36 => vec![plain("Server alert mode enabled.")],
		37 => vec![plain("Server alert mode disabled.")],
		38 => vec![plain("A server raid was reported.")],
		39 => vec![plain("A server incident was reported as a false alarm.")],
		44 => vec![plain("Purchase notification.")],
		46 => vec![plain("Poll results.")],
		55 => vec![actor(), plain(" upgraded the stream to HD.")],
		58 => vec![actor(), plain(" deleted a reported message.")],
		59 => vec![actor(), plain(" timed out "), target(), plain(".")],
		60 => vec![actor(), plain(" kicked "), target(), plain(".")],
		61 => vec![actor(), plain(" banned "), target(), plain(".")],
		62 => vec![actor(), plain(" resolved a report.")],
		65 => vec![actor(), plain(" started a voice hangout.")],
		67 => vec![actor(), plain(" accepted your friend request.")],
		_ => return None,
	};
	Some(SystemMessage {
		segments,
		content_shown,
	})
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::Id;

	fn message(kind: u8) -> Message {
		Message {
			flags: 0,
			components: vec![],
			application_id: None,
			ephemeral: false,
			kind,
			author: User {
				primary_guild: None,
				id: Id(3),
				name: "Robin".into(),
				avatar: None,
				webhook: false,
				kind: Default::default(),
				discriminator: 0,
			},
			mentions: vec![User {
				primary_guild: None,
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

	#[test]
	fn newer_service_types_keep_names_actionable_and_missing_targets_honest() {
		for (kind, expected) in [
			(30, "Robin requested to speak."),
			(55, "Robin upgraded the stream to HD."),
			(58, "Robin deleted a reported message."),
			(59, "Robin timed out Casey."),
			(60, "Robin kicked Casey."),
			(61, "Robin banned Casey."),
			(62, "Robin resolved a report."),
			(65, "Robin started a voice hangout."),
			(67, "Robin accepted your friend request."),
		] {
			let mut message = message(kind);
			message.content.clear();
			let system = message.system_message().unwrap();
			assert_eq!(message.display_text(), expected);
			assert_eq!(system.segments[0].user.as_ref().unwrap().id, Id(3));
			assert!(!system.content_shown);
			if matches!(kind, 59..=61) {
				assert_eq!(system.segments[2].user.as_ref().unwrap().id, Id(4));
				message.mentions.clear();
				assert!(message.display_text().contains("a member"));
			}
		}
	}
	#[test]
	fn names_and_values_are_strong_and_clickable() {
		let join = message(7).system_message().unwrap();
		assert_eq!(join.summary(), "Welcome, Robin! Joined the server.");
		assert_eq!(join.segments[1].user.as_ref().map(|u| u.id), Some(Id(3)));
		assert!(join.segments[1].strong && !join.segments[0].strong);
		assert!(!join.content_shown);

		let rename = message(4);
		let system = rename.system_message().unwrap();
		assert_eq!(system.summary(), "Robin changed the channel name: general");
		assert!(system.content_shown);
		assert_eq!(
			rename.display_text(),
			"Robin changed the channel name: general"
		);
		let last = system.segments.last().unwrap();
		assert!(last.strong && last.user.is_none());

		let mut blank = message(18);
		blank.content = "  ".into();
		let system = blank.system_message().unwrap();
		assert_eq!(system.summary(), "Robin started a thread.");
		assert!(!system.content_shown);

		let add = message(1).system_message().unwrap();
		assert_eq!(add.summary(), "Robin added Casey to the conversation.");
		assert_eq!(add.segments[2].user.as_ref().map(|u| u.id), Some(Id(4)));

		let mut long = message(6);
		long.author.name = "界".repeat(1000);
		assert!(long.system_summary().unwrap().chars().count() < 150);
	}
}
