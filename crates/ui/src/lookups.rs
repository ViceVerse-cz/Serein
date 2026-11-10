//! Requests from deep widgets to read an unknown person or an unloaded reply, collected per
//! frame and turned into commands once by the messaging view.
use client_core::{Command, State};
use model::Id;

const KEY: &str = "lookup-requests";
/// Requests kept per frame, only to bound memory. Most are already kept, failed or not
/// readable, so the state's own in-flight limit, not this queue, decides what is asked for.
const PER_FRAME: usize = 64;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Request {
	User(Id),
	Message { channel: Id, message: Id },
}

/// Asks the messaging view to read `request` this frame.
pub(crate) fn request(ctx: &egui::Context, request: Request) {
	ctx.data_mut(|data| {
		let queue = data.get_temp_mut_or_default::<Vec<Request>>(egui::Id::unique(KEY));
		if queue.len() < PER_FRAME && !queue.contains(&request) {
			queue.push(request);
		}
	});
}

/// Turns this frame's requests into lookups the state still admits.
pub(crate) fn drain(ctx: &egui::Context, state: &mut State, commands: &mut Vec<Command>) {
	let requests = ctx.data_mut(|data| data.remove_temp::<Vec<Request>>(egui::Id::unique(KEY)));
	for request in requests.unwrap_or_default() {
		let command = match request {
			Request::User(user) => state.prepare_user_lookup(user),
			Request::Message { channel, message } => state.prepare_message_lookup(channel, message),
		};
		commands.extend(command);
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	/// Verifies duplicate lookups are dropped, in-flight lookups are capped, refused requests never
	/// crowd out admitted ones, and each frame's requests are drained only once.
	#[test]
	fn requests_are_bounded_deduplicated_and_drained_once() {
		let ctx = egui::Context::default();
		let mut state = State {
			auth: client_core::auth::AuthState::Authenticated,
			..Default::default()
		};
		for id in [5, 5, 6, 7, 8, 9] {
			request(&ctx, Request::User(Id(id)));
		}
		let mut commands = vec![];
		drain(&ctx, &mut state, &mut commands);
		assert_eq!(
			commands.len(),
			4,
			"duplicates are dropped and at most four lookups are in flight"
		);
		// Requests the state refuses never crowd out ones it still admits.
		let mut state = State {
			auth: client_core::auth::AuthState::Authenticated,
			..Default::default()
		};
		for id in 0..PER_FRAME as u64 - 1 {
			request(
				&ctx,
				Request::Message {
					channel: Id(1),
					message: Id(id + 1),
				},
			);
		}
		request(&ctx, Request::User(Id(5)));
		let mut admitted = vec![];
		drain(&ctx, &mut state, &mut admitted);
		assert_eq!(admitted.len(), 1, "the readable request still runs");
		drain(&ctx, &mut state, &mut admitted);
		assert_eq!(admitted.len(), 1, "a frame's requests are drained once");
	}
}
