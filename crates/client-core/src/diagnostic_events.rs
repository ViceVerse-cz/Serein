//! Central result classification. Never format events: they contain private identifiers/content.
use crate::*;

pub(crate) fn record(event: &Event) {
	if !diagnostics::is_enabled(diagnostics::Level::Debug) {
		return;
	}
	let Some((operation, reason)) = classify(event) else {
		return;
	};
	diagnostics::record_fields(
		diagnostics::Category::Api,
		diagnostics::Level::Error,
		"operation failed",
		&[("operation", operation.into()), ("reason", reason.into())],
	);
}

fn classify(event: &Event) -> Option<(&'static str, &'static str)> {
	let (operation, failure) = match event {
		Event::Sticker {
			result: Err(failure),
			..
		} => ("Sticker", *failure),
		Event::ApplicationCommands {
			result: Err(failure),
			..
		} => ("ApplicationCommands", *failure),
		Event::MemberSearch {
			result: Err(failure),
			..
		} => ("MemberSearch", *failure),
		Event::MessagingPermissions {
			result: Err(failure),
			..
		} => ("MessagingPermissions", *failure),
		Event::JoinInvite {
			result: Err(failure),
			..
		} => ("JoinInvite", *failure),
		Event::GuildCreated {
			result: Err(failure),
			..
		} => ("GuildCreated", *failure),
		Event::Invite {
			result: Err(failure),
			..
		} => ("Invite", *failure),
		Event::PostCreated {
			result: Err(failure),
			..
		} => ("PostCreated", *failure),
		Event::Archives {
			result: Err(failure),
			..
		} => ("Archives", *failure),
		Event::ThreadStarter {
			result: Err(failure),
			..
		} => ("ThreadStarter", *failure),
		Event::ForumPosts {
			result: Err(failure),
			..
		} => ("ForumPosts", *failure),
		Event::Search {
			result: Err(failure),
			..
		} => ("Search", *failure),
		Event::Gifs {
			result: Err(failure),
			..
		} => ("Gifs", *failure),
		Event::GifFavorites {
			result: Err(failure),
			..
		} => ("GifFavorites", *failure),
		Event::Profile {
			result: Err(failure),
			..
		} => ("Profile", *failure),
		Event::StreamPreview {
			result: Err(failure),
			..
		} => ("StreamPreview", *failure),
		Event::BadgeSettings {
			result: Err(failure),
			..
		} => ("BadgeSettings", *failure),
		Event::ProfileEdited {
			result: Err(failure),
			..
		} => ("ProfileEdited", *failure),
		Event::Edited {
			result: Err(failure),
			..
		} => ("Edited", *failure),
		Event::Pinned {
			result: Err(failure),
			..
		} => ("Pinned", *failure),
		Event::SendResult {
			result: Err(failure),
			..
		} => ("SendResult", *failure),
		Event::ChannelAction(channel_actions::Event::Finished {
			result: Err(failure),
			..
		}) => ("channel_actions_Finished", *failure),
		Event::ServerAdmin(server_admin::Event {
			result: Err(failure),
			..
		}) => ("server_admin", *failure),
		Event::ServerSettings(server_settings::Event {
			result: Err(failure),
			..
		}) => ("server_settings", *failure),
		Event::Onboarding(onboarding::Event::Loaded {
			result: Err(failure),
			..
		}) => ("onboarding_Loaded", *failure),
		Event::Onboarding(onboarding::Event::Submitted {
			result: Err(failure),
			..
		}) => ("onboarding_Submitted", *failure),
		Event::UserAction(user_actions::Event::DmOpened {
			result: Err(failure),
			..
		}) => ("user_actions_DmOpened", *failure),
		Event::UserAction(user_actions::Event::GroupCreated {
			result: Err(failure),
			..
		}) => ("user_actions_GroupCreated", *failure),
		Event::UserAction(user_actions::Event::NoteLoaded {
			result: Err(failure),
			..
		}) => ("user_actions_NoteLoaded", *failure),
		Event::UserAction(user_actions::Event::Written {
			result: Err(failure),
			..
		}) => ("user_actions_Written", *failure),
		Event::ServerAction(server_actions::Event::InviteSent {
			result: Err(failure),
			..
		}) => ("server_actions_InviteSent", *failure),
		Event::ServerAction(server_actions::Event::Written {
			result: Err(failure),
			..
		}) => ("server_actions_Written", *failure),
		Event::GroupAction(group_actions::Event::Written {
			result: Err(failure),
			..
		}) => ("group_actions_Written", *failure),
		Event::Interaction(interactions::Event::Submitted {
			result: Err(failure),
			..
		}) => ("interactions_Submitted", *failure),
		Event::Reactions(reactions::Event::Read {
			result: Err(failure),
			..
		}) => ("reactions_Read", *failure),
		Event::Reactions(reactions::Event::Written {
			result: Err(failure),
			..
		}) => ("reactions_Written", *failure),
		Event::Reactions(reactions::Event::Users {
			result: Err(failure),
			..
		}) => ("reactions_Users", *failure),
		Event::Polls(polls::Event::Result {
			result: Err(failure),
			..
		}) => ("polls_Result", *failure),
		Event::ReadState(read_state::Event::Result {
			result: Err(failure),
			..
		}) => ("read_state_Result", *failure),
		Event::ReadState(read_state::Event::GuildAck {
			result: Err(failure),
			..
		}) => ("read_state_GuildAck", *failure),
		Event::ReadState(read_state::Event::AllAck {
			result: Err(failure),
			..
		}) => ("read_state_AllAck", *failure),
		Event::ForumSummaries { results, .. } => {
			let failure = results
				.iter()
				.find_map(|(_, result)| result.as_ref().err())?;
			("ForumSummaries", *failure)
		}
		Event::Interaction(interactions::Event::Failed { .. }) => {
			("interactions_Failed", auth::Failure::Protocol)
		}
		Event::Failure(failure) => ("session", *failure),
		Event::HistoryFailed { failure, .. } => ("history", *failure),
		Event::StickerPacks(Err(failure)) => ("sticker_packs", *failure),
		Event::GuildFolders(Err(failure)) => ("guild_folders", *failure),
		Event::TypingSent(Err(failure)) => ("typing", *failure),
		_ => return None,
	};
	let reason = match failure {
		auth::Failure::Expired => "expired",
		auth::Failure::Challenged => "challenged",
		auth::Failure::Forbidden => "forbidden",
		auth::Failure::RateLimited => "rate_limited",
		auth::Failure::Network => "network",
		auth::Failure::Ambiguous => "ambiguous",
		auth::Failure::Protocol | auth::Failure::ProtocolAt(_) => "protocol",
		auth::Failure::Capacity | auth::Failure::CapacityAt(_) => "capacity",
		auth::Failure::InvalidCredential => "invalid_credential",
	};
	Some((operation, reason))
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn failure_classification_discards_private_labels_and_operation_identifiers() {
		let send = Event::SendResult {
			nonce: "private nonce or injected message content".into(),
			result: Err(auth::Failure::ProtocolAt("private token or server error")),
		};
		assert_eq!(classify(&send), Some(("SendResult", "protocol")));
		let reaction = Event::Reactions(reactions::Event::Written {
			channel: model::Id(987654321),
			message: model::Id(123456789),
			request: 55,
			result: Err(auth::Failure::CapacityAt("private path or account name")),
		});
		assert_eq!(classify(&reaction), Some(("reactions_Written", "capacity")));
		let summaries = Event::ForumSummaries {
			request: 88,
			results: vec![(
				model::Id(7654321),
				Err(auth::Failure::ProtocolAt("private source")),
			)],
		};
		assert_eq!(classify(&summaries), Some(("ForumSummaries", "protocol")));
		assert_eq!(classify(&Event::TypingSent(Ok(()))), None);
	}
}
