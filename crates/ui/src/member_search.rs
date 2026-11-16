use client_core::{Command, State};
use model::Id;

#[derive(Default)]
pub struct Search {
	pub text: String,
	key: Option<(u64, Id, String)>,
	changed: f64,
	sent: bool,
	prepared: Option<client_core::member_search::Request>,
}

impl Search {
	pub fn sync(
		&mut self,
		ctx: &egui::Context,
		state: &mut State,
		channel: Id,
		slot: usize,
		commands: &mut Vec<Command>,
	) {
		let now = ctx.input(|input| input.time);
		let text = self.text.trim();
		let key = (state.generation, channel, text.to_owned());
		if self.key.as_ref() != Some(&key) {
			let changed_scope = self
				.key
				.as_ref()
				.is_some_and(|old| old.0 != key.0 || old.1 != key.1)
				|| state.member_search[slot]
					.request
					.as_ref()
					.is_some_and(|request| {
						request.channel != channel
							|| state.channel(channel).and_then(|c| c.guild) != Some(request.guild)
					});
			self.key = Some(key);
			self.changed = now;
			self.sent = false;
			self.prepared = None;
			if changed_scope {
				state.member_search[slot] = Default::default();
			}
		}
		if self.prepared.is_some() && state.member_search[slot].request.is_none() {
			self.prepared = None;
			self.sent = false;
		}
		if text.is_empty() || self.sent || state.channel(channel).and_then(|c| c.guild).is_none() {
			return;
		}
		if self.prepared.is_none() {
			let Some(Command::MemberSearchLocal(request)) =
				state.prepare_member_search(channel, text, slot)
			else {
				return;
			};
			self.prepared = Some(request.clone());
			commands.push(Command::MemberSearchLocal(request));
		}
		if now - self.changed >= 0.35 {
			if let Some(command) = state.dispatch_member_search(self.prepared.as_ref().unwrap()) {
				commands.push(command);
				self.sent = true;
			}
		} else {
			ctx.request_repaint_after(std::time::Duration::from_secs_f64(
				(0.35 - (now - self.changed)).clamp(0.0, 0.35),
			));
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn frame(
		search: &mut Search,
		ctx: &egui::Context,
		state: &mut State,
		channel: Id,
		time: f64,
	) -> Vec<Command> {
		let mut commands = vec![];
		ctx.run_ui(
			egui::RawInput {
				time: Some(time),
				..Default::default()
			},
			|_| search.sync(ctx, state, channel, 0, &mut commands),
		)
		.drop_without_applying_deltas();
		commands
	}

	#[test]
	fn sqlite_suggestions_arrive_before_the_debounced_live_query() {
		let mut state = test_support::demo_state();
		state.demo = false;
		state.auth = client_core::auth::AuthState::Authenticated;
		state.gateway_connected = true;
		let channel = state.selected.unwrap();
		let guild = state.channel(channel).unwrap().guild.unwrap();
		let account = state.user.as_ref().unwrap().id;
		let mut store = local_store::LocalStore::open(std::path::Path::new(":memory:")).unwrap();
		store
			.save_member_displays(
				account,
				&[model::MemberDisplay {
					guild,
					user: Id(900000),
					name: "Global display".into(),
					username: Some("account_name".into()),
					nick: Some("Cached nickname".into()),
				}],
			)
			.unwrap();
		let ctx = egui::Context::default();
		let mut search = Search {
			text: "Cached".into(),
			..Default::default()
		};
		let mut commands = frame(&mut search, &ctx, &mut state, channel, 0.0);
		assert_eq!(commands.len(), 1);
		let Command::MemberSearchLocal(request) = commands.pop().unwrap() else {
			panic!("only a local lookup may start before the debounce");
		};
		assert!(state.member_search[0].local_only);
		let rows = store
			.search_member_displays(account, guild, &request.query, 100)
			.unwrap();
		assert!(state.hydrate_member_search_displays(
			&request,
			state.member_display_revision(),
			rows
		));
		assert_eq!(
			state.member_nickname(channel, Id(900000)),
			Some(Some("Cached nickname"))
		);
		assert!(!state.member_search[0].finished);
		assert!(state.member_search[0].cooldown_until.is_none());
		assert!(frame(&mut search, &ctx, &mut state, channel, 0.349).is_empty());
		let commands = frame(&mut search, &ctx, &mut state, channel, 0.351);
		assert!(matches!(&commands[..], [Command::MemberSearch(live)] if live == &request));
		assert!(!state.member_search[0].local_only);
		assert!(frame(&mut search, &ctx, &mut state, channel, 1.0).is_empty());
	}

	#[test]
	fn rapid_queries_only_dispatch_the_latest_live_request() {
		let mut state = test_support::demo_state();
		state.demo = false;
		state.auth = client_core::auth::AuthState::Authenticated;
		state.gateway_connected = true;
		let channel = state.selected.unwrap();
		let ctx = egui::Context::default();
		let mut search = Search {
			text: "First".into(),
			..Default::default()
		};
		let mut commands = frame(&mut search, &ctx, &mut state, channel, 0.0);
		let Command::MemberSearchLocal(first) = commands.pop().unwrap() else {
			panic!("first local query");
		};
		for (text, time) in [("Second", 0.1), ("Third", 0.2)] {
			search.text = text.into();
			let commands = frame(&mut search, &ctx, &mut state, channel, time);
			assert!(
				matches!(&commands[..], [Command::MemberSearchLocal(request)] if request.query == text)
			);
		}
		assert!(!state.hydrate_member_search_displays(
			&first,
			state.member_display_revision(),
			vec![]
		));
		assert!(frame(&mut search, &ctx, &mut state, channel, 0.4).is_empty());
		let commands = frame(&mut search, &ctx, &mut state, channel, 0.551);
		assert!(
			matches!(&commands[..], [Command::MemberSearch(request)] if request.query == "Third")
		);
		assert!(frame(&mut search, &ctx, &mut state, channel, 0.9).is_empty());
	}

	#[test]
	fn competing_search_owners_do_not_reprepare_each_others_queries() {
		let mut state = test_support::demo_state();
		state.demo = false;
		state.auth = client_core::auth::AuthState::Authenticated;
		state.gateway_connected = true;
		let channel = state.selected.unwrap();
		let ctx = egui::Context::default();
		let mut first = Search {
			text: "First".into(),
			..Default::default()
		};
		let mut second = Search {
			text: "Second".into(),
			..Default::default()
		};
		assert!(matches!(
			&frame(&mut first, &ctx, &mut state, channel, 0.0)[..],
			[Command::MemberSearchLocal(_)]
		));
		assert!(matches!(
			&frame(&mut second, &ctx, &mut state, channel, 0.1)[..],
			[Command::MemberSearchLocal(_)]
		));
		assert!(frame(&mut first, &ctx, &mut state, channel, 0.4).is_empty());
		assert!(
			matches!(&frame(&mut second, &ctx, &mut state, channel, 0.5)[..], [Command::MemberSearch(request)] if request.query == "Second")
		);
		for time in [0.6, 0.8, 1.0] {
			assert!(frame(&mut first, &ctx, &mut state, channel, time).is_empty());
			assert!(frame(&mut second, &ctx, &mut state, channel, time).is_empty());
		}
		first.text = "Third".into();
		assert!(
			matches!(&frame(&mut first, &ctx, &mut state, channel, 1.1)[..], [Command::MemberSearchLocal(request)] if request.query == "Third")
		);
		assert!(frame(&mut second, &ctx, &mut state, channel, 1.2).is_empty());
		assert!(
			matches!(&frame(&mut first, &ctx, &mut state, channel, 1.5)[..], [Command::MemberSearch(request)] if request.query == "Third")
		);
		assert!(frame(&mut second, &ctx, &mut state, channel, 1.6).is_empty());
		state.member_search[0] = Default::default();
		assert!(
			matches!(&frame(&mut first, &ctx, &mut state, channel, 2.0)[..], [Command::MemberSearchLocal(local), Command::MemberSearch(live)] if local == live && local.query == "Third")
		);
	}
}
