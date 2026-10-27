//! One bounded "start a conversation" picker: a direct message, or a group of up to ten people.
use crate::{avatars::Avatars, design, dialog};
use client_core::{Command, State, user_actions::MAX_GROUP_RECIPIENTS};
use model::Id;

/// Friends listed at once; searching narrows the rest.
const FRIENDS: usize = 16;

#[derive(Default)]
pub(crate) struct NewMessageUi {
	open: bool,
	generation: u64,
	query: String,
	selected: Vec<Id>,
	focus: bool,
	/// Friends for `queried`: the query, the friend view it was built for, and when.
	queried: Option<(String, u64, f64)>,
	matches: Vec<Id>,
}

/// The friends matching `query` by display name or handle, alphabetical and bounded.
fn matching_friends(state: &State, query: &str) -> Vec<Id> {
	let query = query.to_lowercase();
	let words: Vec<&str> = query.split_whitespace().collect();
	let mut found: Vec<(String, Id)> = Vec::new();
	for friend in state.friends() {
		let name = state.user_display_name(friend).to_lowercase();
		let handle = state
			.friend_username(friend.id)
			.unwrap_or_default()
			.to_lowercase();
		if words
			.iter()
			.all(|word| name.contains(word) || handle.contains(word))
		{
			found.push((name, friend.id));
		}
	}
	found.sort_by(|left, right| left.0.cmp(&right.0).then_with(|| left.1.0.cmp(&right.1.0)));
	found.truncate(FRIENDS);
	found.into_iter().map(|(_, id)| id).collect()
}

impl NewMessageUi {
	/// Opens the picker for the signed-in session; a stale dialog is dropped on account change.
	pub fn open(&mut self, state: &State) {
		if state.user.is_none() && !state.demo {
			return;
		}
		*self = Self {
			open: true,
			generation: state.generation,
			focus: true,
			..Self::default()
		};
	}
	pub fn show(
		&mut self,
		ctx: &egui::Context,
		state: &mut State,
		avatars: &mut Avatars,
		commands: &mut Vec<Command>,
	) {
		if !self.open {
			return;
		}
		if self.generation != state.generation {
			*self = Self::default();
			return;
		}
		// Someone unfriended while the picker is open can no longer be messaged from it.
		self.selected.retain(|user| state.friend(*user).is_some());
		let mut create = false;
		let mut close = false;
		let pending = state.user_action_pending();
		let response =
			dialog::Dialog::new("new-message", crate::i18n::translate("new-message-title"))
				.subtitle(crate::i18n::translate("new-message-subtitle"))
				.width(440.0)
				.show(ctx, |d| {
					d.content(|ui| {
						let input = design::input(
							ui,
							egui::TextEdit::singleline(&mut self.query)
								.align(egui::Align2::LEFT_CENTER)
								.hint_text(crate::i18n::translate("new-message-search"))
								.char_limit(128),
						);
						if std::mem::take(&mut self.focus) {
							input.request_focus();
						}
						design::hint(ui, &crate::i18n::translate("new-message-hint"));
						ui.add_space(8.0);
					});
					d.content(|ui| {
						egui::ScrollArea::vertical()
							.id_salt("new-message-people")
							.max_height((ctx.content_rect().height() - 340.0).clamp(80.0, 300.0))
							.auto_shrink([false, true])
							.show(ui, |ui| {
								// Rebuild at most once a second, like the other search lists.
								let now = ui.input(|input| input.time);
								let view = state.relationship_view();
								let stale =
									self.queried.as_ref().is_none_or(|(query, cached, at)| {
										*query != self.query
											|| *cached != view || !(0.0..1.0).contains(&(now - at))
									});
								if stale {
									let query = self.query.clone();
									self.matches = matching_friends(state, &query);
									self.queried = Some((query, view, now));
								}
								if self.matches.is_empty() {
									ui.label(crate::i18n::translate("new-message-no-people-found"));
								}
								for index in 0..self.matches.len() {
									let id = self.matches[index];
									let Some(friend) = state.friend(id) else {
										continue;
									};
									let selected = self.selected.contains(&id);
									let full = self.selected.len() >= MAX_GROUP_RECIPIENTS;
									let enabled = !pending && (selected || !full);
									let handle = state.friend_username(id).unwrap_or_default();
									let response = ui
										.push_id(id.0, |ui| {
											ui.add_enabled_ui(enabled, |ui| {
												design::selection_row(
													ui,
													selected,
													state.user_display_name(friend),
													&format!("@{handle}"),
													|ui, rect| {
														avatars.paint_user(
															ui, friend, 32.0, rect, state.demo,
														);
													},
												)
											})
											.inner
										})
										.inner;
									if response.clicked() {
										if selected {
											self.selected.retain(|user| *user != id);
										} else {
											self.selected.push(id);
										}
									}
								}
							});
					});
					d.footer(|ui| {
						let ready = !self.selected.is_empty()
							&& !pending && (state.demo || state.gateway_connected);
						create = ui
							.add_enabled_ui(ready, |ui| {
								dialog::action(ui, "new-message-create", dialog::Action::Primary)
							})
							.inner
							.clicked();
						close = dialog::action(ui, "new-message-cancel", dialog::Action::Neutral)
							.clicked();
					});
				});
		if create {
			if let Some(command) = self.issue(state) {
				commands.push(command);
				close = true;
			} else if self.opened(state) {
				// The conversation was already loaded, or already on screen.
				close = true;
			}
		}
		if close || response.close {
			*self = Self::default();
		}
	}
	/// One recipient opens a DM; more become a group the service admits.
	fn issue(&self, state: &mut State) -> Option<Command> {
		if let [user] = self.selected.as_slice() {
			let friend = state.friend(*user)?.clone();
			return state.open_user_dm(&friend);
		}
		state.create_group_dm(&self.selected)
	}
	/// Whether the selected conversation is the one these people would start.
	fn opened(&self, state: &State) -> bool {
		let kind = if self.selected.len() == 1 { 1 } else { 3 };
		state
			.selected
			.and_then(|id| state.channel(id))
			.is_some_and(|channel| {
				channel.guild.is_none()
					&& channel.kind == kind
					&& channel.recipients.len() == self.selected.len()
					&& self
						.selected
						.iter()
						.all(|user| channel.recipients.iter().any(|entry| entry.id == *user))
			})
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use client_core::{Envelope, Event, user_actions};

	fn fixture() -> (State, Vec<model::User>) {
		let mut state = State {
			demo: true,
			user: Some(model::User {
				id: Id(1),
				name: "Signed in".into(),
				avatar: None,
				webhook: false,
				kind: Default::default(),
				discriminator: 0,
				primary_guild: None,
				cosmetics: None,
			}),
			..Default::default()
		};
		let people: Vec<model::User> = (0..2)
			.map(|index| model::User {
				id: Id(20 + index),
				name: format!("Friend {index}"),
				avatar: None,
				webhook: false,
				kind: Default::default(),
				discriminator: 0,
				primary_guild: None,
				cosmetics: None,
			})
			.collect();
		for event in [
			user_actions::Event::Relationships(Some(vec![])),
			user_actions::Event::Friends(Some(
				people
					.iter()
					.map(|user| (user.clone(), "synthetic.handle".into()))
					.collect(),
			)),
		] {
			state.apply(Envelope {
				generation: state.generation,
				event: Event::UserAction(event),
			});
		}
		(state, people)
	}

	#[test]
	fn the_picker_opens_a_friend_and_creates_a_group_from_the_selected_people() {
		let (mut state, people) = fixture();
		let mut picker = NewMessageUi::default();
		picker.open(&state);
		assert!(picker.open && picker.generation == state.generation);
		assert!(picker.issue(&mut state).is_none(), "nothing was chosen");
		picker.selected = vec![people[0].id];
		assert!(matches!(
			picker.issue(&mut state),
			Some(Command::UserAction {
				action: user_actions::Action::OpenDm(user),
				..
			}) if user == people[0].id
		));
		// A second picker run creates the group instead of replacing the DM request.
		let (mut state, people) = fixture();
		let mut picker = NewMessageUi::default();
		picker.open(&state);
		picker.selected = people.iter().map(|user| user.id).collect();
		let Some(Command::UserAction {
			action: user_actions::Action::CreateGroup { recipients },
			..
		}) = picker.issue(&mut state)
		else {
			panic!("expected one group request");
		};
		assert_eq!(
			recipients,
			people.iter().map(|user| user.id).collect::<Vec<_>>()
		);
		// Regression: choosing the group already on screen sends nothing but still closes.
		state.channels.push(model::Channel {
			last_message: None,
			id: Id(50),
			guild: None,
			parent_id: None,
			position: 0,
			name: "Synthetic group".into(),
			kind: 3,
			recipients: people.clone(),
			member_list_id: None,
			tags: None,
			message_count: None,
			icon: None,
		});
		state.invalidate_navigation();
		state.selected = Some(Id(50));
		assert!(picker.issue(&mut state).is_none() && picker.opened(&state));
		picker.selected.truncate(1);
		assert!(!picker.opened(&state), "one person is not this group");
	}
}
