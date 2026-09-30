//! Offline poll action exercise; never sends Discord requests.
use client_core::{
	Command, Event, State,
	polls::{Action, Request, now_ms},
};
use model::{
	Id,
	polls::{Create, Media},
};
pub fn preview() -> State {
	let mut state = test_support::demo_state();
	state.auth = client_core::auth::AuthState::Authenticated;
	state.gateway_connected = true;
	state.freshness = model::Freshness::Fresh;
	let channel = state.selected.unwrap();
	let create = draft();
	let mut message = test_support::message(99101, channel);
	message.content.clear();
	message.author = state.user.clone().unwrap();
	message.poll = Some(Box::new(create.fixture(now_ms())));
	message.poll.as_mut().unwrap().answers[0].votes = 3;
	message.poll.as_mut().unwrap().answers[1].votes = 2;
	message.extra_content.poll = true;
	state.timeline.clear();
	state.timeline.insert(message, true, false).unwrap();
	state
}
fn draft() -> Create {
	Create {
		question: "What should we play tonight?".into(),
		answers: ["Minecraft", "Stardew Valley", "Terraria"]
			.into_iter()
			.map(|s| Media {
				text: s.into(),
				emoji: None,
			})
			.collect(),
		duration: 24,
		multiselect: false,
	}
}
pub fn respond(state: &State, request: Request, synthetic_id: &mut u64) -> Event {
	let result = match request.action {
		Action::Create(create) => {
			*synthetic_id += 1;
			let mut message = test_support::message(*synthetic_id, request.channel);
			message.content.clear();
			message.author = state.user.clone().unwrap();
			message.nonce = Some(request.nonce);
			message.poll = Some(Box::new(create.fixture(now_ms())));
			message.extra_content.poll = true;
			Ok(message)
		}
		action => state
			.timeline
			.get(request.message.unwrap())
			.cloned()
			.ok_or(client_core::auth::Failure::Protocol)
			.map(|mut message| {
				let poll = message.poll.as_mut().unwrap();
				match action {
					Action::Vote(ids) => {
						for answer in &mut poll.answers {
							let me = ids.contains(&answer.id);
							if answer.me != me {
								answer.votes = if me {
									answer.votes + 1
								} else {
									answer.votes.saturating_sub(1)
								};
							}
							answer.me = me;
						}
					}
					Action::End => {
						poll.finalized = true;
						poll.expiry = Some(now_ms());
					}
					_ => {}
				}
				message
			}),
	};
	Event::Polls(client_core::polls::Event::Result {
		channel: request.channel,
		message: request.message,
		request: request.request,
		result,
	})
}
pub fn check() {
	let mut state = preview();
	let id = Id(99101);
	assert!(!state.can_edit(state.selected.unwrap(), id));
	let mut synthetic_id = 99200;
	let generation = state.generation;
	let Command::Polls(request) = state.prepare_poll(Some(id), Action::Vote(vec![1])).unwrap()
	else {
		panic!()
	};
	state.apply(client_core::Envelope {
		generation,
		event: respond(&state, request, &mut synthetic_id),
	});
	assert!(
		state
			.timeline
			.get(id)
			.unwrap()
			.poll
			.as_ref()
			.unwrap()
			.answers[0]
			.me
	);
	assert_eq!(
		state
			.timeline
			.get(id)
			.unwrap()
			.poll
			.as_ref()
			.unwrap()
			.answers[0]
			.votes,
		4
	);
	let Command::Polls(request) = state.prepare_poll(Some(id), Action::Vote(vec![])).unwrap()
	else {
		panic!()
	};
	state.apply(client_core::Envelope {
		generation,
		event: respond(&state, request, &mut synthetic_id),
	});
	assert!(
		!state
			.timeline
			.get(id)
			.unwrap()
			.poll
			.as_ref()
			.unwrap()
			.answers[0]
			.me
	);
	assert!(
		state
			.prepare_poll(Some(id), Action::Vote(vec![1, 2]))
			.is_none()
	);
	let Command::Polls(request) = state.prepare_poll(Some(id), Action::End).unwrap() else {
		panic!()
	};
	state.apply(client_core::Envelope {
		generation,
		event: respond(&state, request, &mut synthetic_id),
	});
	assert!(
		state
			.timeline
			.get(id)
			.unwrap()
			.poll
			.as_ref()
			.unwrap()
			.finalized
	);
	assert!(
		state
			.prepare_poll(Some(id), Action::Vote(vec![1]))
			.is_none()
	);
	assert!(draft().valid());
	let mut invalid = draft();
	invalid.answers.clear();
	assert!(!invalid.valid());
	let payload = serde_json::json!({"id":"99500","channel_id":"20","author":{"id":"1","username":"Synthetic"},
        "poll":{"question":{"text":"Choose"},"answers":[{"answer_id":1,"poll_media":{"text":"Yes"}},
        {"answer_id":2,"poll_media":{"emoji":{"name":"yes"}}}],"allow_multiselect":true,"layout_type":1,
        "expiry":"2026-09-30T12:00:00Z","results":{"is_finalized":false,"answer_counts":[{"id":1,"count":3,"me_voted":true}]}}});
	let decoded = discord_protocol::decode::<discord_protocol::MessageDto>(
		&serde_json::to_vec(&payload).unwrap(),
	)
	.unwrap()
	.into_model();
	let poll = decoded.poll.as_ref().unwrap();
	assert_eq!(poll.expiry, Some(1790769600000));
	assert!(poll.answers[0].me);
	assert_eq!(poll.answers[1].votes, 0);
	let mut without_results = payload.clone();
	without_results["poll"]
		.as_object_mut()
		.unwrap()
		.remove("results");
	let mut next = discord_protocol::decode::<discord_protocol::MessageDto>(
		&serde_json::to_vec(&without_results).unwrap(),
	)
	.unwrap()
	.into_model();
	next.poll.as_mut().unwrap().retain_results(poll);
	assert_eq!(next.poll.as_ref(), Some(poll));
	let patch = discord_protocol::decode::<discord_protocol::PatchDto>(
		br#"{"id":"99500","channel_id":"20","poll":null}"#,
	)
	.unwrap()
	.into_model();
	state.timeline.insert(next, false, false).unwrap();
	state.timeline.patch(patch).unwrap();
	assert!(state.timeline.get(Id(99500)).unwrap().poll.is_none());
	let patch = discord_protocol::decode::<discord_protocol::PatchDto>(
		br#"{"id":"99500","channel_id":"20"}"#,
	)
	.unwrap()
	.into_model();
	state
		.timeline
		.insert(decoded.clone(), false, false)
		.unwrap();
	state.timeline.patch(patch).unwrap();
	assert_eq!(state.timeline.get(Id(99500)).unwrap().poll, decoded.poll);
	let Command::Polls(request) = state
		.prepare_poll(None, Action::Create(Box::new(draft())))
		.unwrap()
	else {
		panic!()
	};
	state.apply(client_core::Envelope {
		generation,
		event: respond(&state, request, &mut synthetic_id),
	});
	assert_eq!(state.polls.created, 1);
	ui::debug_poll_check(&state);
	println!("Offline polls: vote/remove/end, validation and themed card layout passed");
}
