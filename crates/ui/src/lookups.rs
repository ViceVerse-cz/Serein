//! Requests from deep widgets to read an unknown person or an unloaded reply, collected per
//! frame and turned into commands once by the messaging view.
use client_core::{Command, State};
use model::Id;

const KEY: &str = "lookup-requests";
/// Requests kept per frame; more are dropped and asked for again on a later hover.
const PER_FRAME: usize = 4;

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
			PER_FRAME,
			"duplicates and overflow are dropped"
		);
		drain(&ctx, &mut state, &mut commands);
		assert_eq!(
			commands.len(),
			PER_FRAME,
			"a frame's requests are drained once"
		);
	}
}
