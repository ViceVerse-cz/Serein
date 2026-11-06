//! Offline message-link showcase, including exact target pages for native interaction checks.
use crate::{message, permission_snapshot};
use client_core::{Envelope, Event, State};
use model::{Freshness, Guild, Id, Message};

const FIRST: u64 = 1_100;

fn records(channel: Id) -> Vec<Message> {
	let texts: Vec<String> = if channel == Id(20) {
		vec![
			"Message links · synthetic, offline conversations. Click a chip to jump; no browser, account or microphone is used.".into(),
			format!("Same channel: https://discord.com/channels/10/20/{FIRST}"),
			"Other channel: https://discord.com/channels/10/21/100\nThread: https://discord.com/channels/10/28/100 · Forum post: https://discord.com/channels/10/27/100".into(),
			"Other server: https://discord.com/channels/30/31/100\nDirect message: https://discord.com/channels/@me/22/100 · Group: https://discord.com/channels/@me/29/100".into(),
			"Old message: https://discord.com/channels/10/20/100 · Missing message: https://discord.com/channels/10/20/999\nUnknown channel: https://discord.com/channels/10/9999/100".into(),
			"Long name: https://discord.com/channels/10/32/100\nMasked label: [Read the original](https://discord.com/channels/10/21/100)".into(),
			"Code stays literal: `https://discord.com/channels/10/21/100`\nHidden until revealed: ||https://discord.com/channels/10/28/100||".into(),
		]
	} else {
		vec![
			"Original message · this is the exact synthetic message-link destination.".into(),
			"Context below the original. This page is generated locally, never fetched from Discord.".into(),
			format!("Back to the showcase: https://discord.com/channels/10/20/{FIRST}"),
		]
	};
	texts
		.into_iter()
		.enumerate()
		.map(|(index, content)| {
			let id = if channel == Id(20) {
				FIRST + index as u64
			} else {
				100 + index as u64
			};
			let mut record = message(id, channel);
			record.content = content;
			record.embeds.clear();
			record.attachments.clear();
			record.reactions = Some(vec![]);
			record
		})
		.collect()
}

/// New fixture shared by the before/after capture; not a live Discord conversation.
pub fn message_links_demo_state() -> State {
	let mut state = crate::chat_demo_state();
	let base = state.channel(Id(20)).unwrap().clone();
	state.guilds.push(Guild {
		id: Id(30),
		name: "Another synthetic server".into(),
		icon: None,
		emojis: None,
		stickers: None,
		default_message_notifications: None,
	});
	for (id, guild, name) in [
		(31, 30, "project-updates"),
		(
			32,
			10,
			"a-very-long-conversation-name-to-check-that-inline-message-links-fit-small-windows",
		),
	] {
		let mut channel = base.clone();
		channel.id = Id(id);
		channel.guild = Some(Id(guild));
		channel.parent_id = None;
		channel.name = name.into();
		channel.last_message = Some(Id(102));
		state.channels.push(channel);
	}
	state
		.permissions
		.replace(permission_snapshot(&state))
		.unwrap();
	state.invalidate_navigation();
	state.timeline.clear();
	for record in records(Id(20)) {
		state.timeline.insert(record, false, false).unwrap();
	}
	state.history_pending = false;
	state.history_targeted = false;
	state.search_target = None;
	state.freshness = Freshness::Fresh;
	state.older_exhausted = true;
	state.channels.iter_mut().for_each(|channel| {
		if channel.id == Id(20) {
			channel.last_message = Some(Id(FIRST + 6));
		}
	});
	state.status = "Offline message-link fixture · no account or network access";
	state.revision += 1;
	state
}

/// Apply a bounded exact synthetic history page, including a deliberately absent target (999).
pub fn load_message_link_page(
	state: &mut State,
	channel: Id,
	request: u64,
	before: Option<Id>,
	after: Option<Id>,
) {
	assert!(before.is_none() || after.is_none());
	let mut messages = records(channel);
	if channel == Id(20) {
		let mut old = message(100, channel);
		old.content = "Original old message · outside the showcase's initial window.".into();
		old.embeds.clear();
		old.attachments.clear();
		messages.insert(0, old);
	}
	messages.retain(|message| {
		before.is_none_or(|before| message.id < before)
			&& after.is_none_or(|after| message.id > after)
	});
	state.apply(Envelope {
		generation: state.generation,
		event: Event::History {
			channel,
			request,
			older: before.is_some(),
			messages,
		},
	});
}

#[cfg(test)]
mod tests {
	use super::*;
	use client_core::Command;

	#[test]
	fn showcase_is_offline_and_loaded_target_needs_no_request() {
		let mut state = message_links_demo_state();
		assert!(state.demo);
		assert_eq!(state.timeline.len(), 7);
		assert!(
			state
				.open_chat_link(Some(Id(10)), Id(20), Some(Id(FIRST)))
				.unwrap()
				.is_none()
		);
		assert_eq!(state.search_target, Some(Id(FIRST)));
	}

	#[test]
	fn showcase_exact_target_pages_and_missing_message_are_bounded() {
		for (guild, channel) in [
			(Some(Id(10)), Id(21)),
			(Some(Id(10)), Id(28)),
			(Some(Id(10)), Id(27)),
			(Some(Id(30)), Id(31)),
			(None, Id(22)),
			(None, Id(29)),
		] {
			let mut state = message_links_demo_state();
			let Some(Command::History {
				request,
				before,
				after,
				..
			}) = state.open_chat_link(guild, channel, Some(Id(100))).unwrap()
			else {
				panic!("fixture target needs history")
			};
			load_message_link_page(&mut state, channel, request, before, after);
			assert_eq!(state.selected, Some(channel));
			assert!(state.timeline.get(Id(100)).is_some());
			assert!(state.timeline.len() <= 50);
			assert!(state.timeline.bytes() <= 4 * 1024 * 1024);
			assert!(state.voice.active.is_none());
		}
		let mut state = message_links_demo_state();
		let Some(Command::History {
			channel,
			request,
			before,
			after,
		}) = state
			.open_chat_link(Some(Id(10)), Id(20), Some(Id(999)))
			.unwrap()
		else {
			panic!("missing fixture target needs history")
		};
		load_message_link_page(&mut state, channel, request, before, after);
		assert!(state.timeline.get(Id(999)).is_none());
	}

	#[test]
	fn queued_fixture_history_keeps_its_original_channel_and_request() {
		let mut state = message_links_demo_state();
		let Some(Command::History {
			channel,
			request,
			before,
			after,
		}) = state
			.open_chat_link(Some(Id(10)), Id(20), Some(Id(100)))
			.unwrap()
		else {
			panic!("old target needs history")
		};
		let Some(Command::History {
			channel: current_channel,
			request: current_request,
			before: current_before,
			after: current_after,
		}) = state
			.open_chat_link(Some(Id(10)), Id(21), Some(Id(100)))
			.unwrap()
		else {
			panic!("new destination needs history")
		};
		load_message_link_page(&mut state, channel, request, before, after);
		assert_eq!(state.selected, Some(current_channel));
		assert_eq!(state.request, current_request);
		assert!(state.history_pending);
		assert!(state.timeline.get(Id(100)).is_none());
		load_message_link_page(
			&mut state,
			current_channel,
			current_request,
			current_before,
			current_after,
		);
		assert!(!state.history_pending);
		assert_eq!(
			state.timeline.get(Id(100)).unwrap().channel,
			current_channel
		);

		// A queued response can also arrive after the selection has been cleared.
		state.selected = None;
		load_message_link_page(&mut state, channel, request, before, after);
		assert_eq!(state.selected, None);
		assert_eq!(state.request, current_request);
		assert_eq!(
			state.timeline.get(Id(100)).unwrap().channel,
			current_channel
		);
	}
}
