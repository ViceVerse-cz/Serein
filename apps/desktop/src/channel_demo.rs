//! Synthetic channel responses; this module never reaches a network adapter.
use client_core::{
	Event, State,
	auth::Failure,
	channel_actions::{Action, Edit, Mute, Outcome, PostDetails},
};
use model::Id;

pub fn execute(
	state: &State,
	guild: Id,
	channel: Id,
	request: u64,
	action: Action,
	next_id: &mut u64,
) -> Event {
	let result = state
		.channel(channel)
		.ok_or(Failure::Forbidden)
		.map(|source| match action {
			Action::Move { .. } => Outcome::Moved,
			Action::Load => {
				Outcome::Details(state.channel_details(channel).cloned().unwrap_or_else(|| {
					Edit {
						name: source.name.clone(),
						overwrites: state
							.permissions
							.channels
							.get(&channel)
							.and_then(|p| p.overwrites.clone())
							.unwrap_or_default(),
						..Edit::default()
					}
				}))
			}
			action @ (Action::PostLoad
			| Action::PostFollow(_)
			| Action::PostArchive(_)
			| Action::PostLock(_)
			| Action::PostRename(_)
			| Action::PostPin(_)
			| Action::PostMute(_)
			| Action::PostNotifications(_)) => {
				let mut details = state.post_details(channel).copied().unwrap_or(PostDetails {
					owner: state.user.as_ref().map(|u| u.id),
					level: 3,
					..PostDetails::default()
				});
				let mut updated = source.clone();
				match action {
					Action::PostFollow(value) => details.followed = value,
					Action::PostArchive(value) => details.archived = value,
					Action::PostLock(value) => details.locked = value,
					Action::PostRename(value) => updated.name = value,
					Action::PostPin(value) => details.pinned = value,
					Action::PostNotifications(value) => details.level = value,
					Action::PostMute(value) => {
						details.muted = value != Mute::Unmute;
						details.mute_until = if let Mute::For(seconds) = value {
							Some(
								std::time::SystemTime::now()
									.duration_since(std::time::UNIX_EPOCH)
									.unwrap_or_default()
									.as_secs() as i64 + i64::from(seconds),
							)
						} else {
							None
						};
					}
					_ => {}
				}
				Outcome::Post {
					channel: Box::new(updated),
					details,
				}
			}
			Action::Delete => Outcome::Deleted,
			Action::Mute(mute) => Outcome::Preferences {
				muted: Some(mute != Mute::Unmute),
				level: None,
				mute_until: if let Mute::For(seconds) = mute {
					Some(
						std::time::SystemTime::now()
							.duration_since(std::time::UNIX_EPOCH)
							.unwrap_or_default()
							.as_secs() as i64 + i64::from(seconds),
					)
				} else {
					None
				},
			},
			Action::Notifications(level) => Outcome::Preferences {
				muted: None,
				level: Some(level),
				mute_until: None,
			},
			action => {
				let mut updated = source.clone();
				let permission_source = if matches!(action, Action::Create { .. }) {
					if source.kind == 4 {
						Some(source.id)
					} else {
						source.parent_id
					}
				} else if matches!(action, Action::CreateCategory { .. }) {
					None
				} else {
					Some(source.id)
				};
				if let Action::Create { kind, .. } = &action {
					updated.kind = kind.wire_kind();
					updated.parent_id = if source.kind == 4 {
						Some(source.id)
					} else {
						source.parent_id
					};
				} else if matches!(action, Action::CreateCategory { .. }) {
					updated.kind = 4;
					updated.parent_id = None;
				}
				let mut edited_overwrites = None;
				match action {
					Action::Edit { after, .. } => {
						updated.name = after.name;
						edited_overwrites = Some(after.overwrites);
					}
					Action::Duplicate { name }
					| Action::Create { name, .. }
					| Action::CreateCategory { name } => {
						*next_id += 1;
						updated.id = Id(*next_id);
						updated.name = name;
						updated.last_message = None;
						updated.message_count = None;
						updated.icon = None;
					}
					_ => unreachable!(),
				}
				Outcome::Channel {
					permissions: Some(model::permissions::Channel {
						id: updated.id,
						guild,
						overwrites: edited_overwrites.or_else(|| {
							permission_source
								.and_then(|id| state.permissions.channels.get(&id))
								.and_then(|p| p.overwrites.clone())
								.or(Some(vec![]))
						}),
					}),
					channel: Box::new(updated),
				}
			}
		});
	Event::ChannelAction(client_core::channel_actions::Event::Finished {
		guild,
		channel,
		request,
		result,
	})
}

#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn offline_edit_create_and_delete_use_the_real_reducer() {
		let mut state = test_support::chat_demo_state();
		let mut permissions = test_support::permission_snapshot(&state);
		for guild in &mut permissions.guilds {
			guild.owner = state.user.as_ref().map(|u| u.id);
		}
		state.permissions.replace(permissions).unwrap();
		let mut next_id = 100_000;
		let run = |state: &mut State, channel, action, next_id: &mut u64| {
			let client_core::Command::ChannelAction {
				guild,
				channel,
				request,
				action,
			} = state.request_channel_action(channel, action).unwrap()
			else {
				panic!("channel command")
			};
			let event = execute(state, guild, channel, request, action, next_id);
			state.apply(client_core::Envelope {
				generation: state.generation,
				event,
			});
		};
		run(&mut state, Id(20), Action::Load, &mut next_id);
		let before = state.channel_details(Id(20)).unwrap().clone();
		let after = Edit {
			name: "renamed".into(),
			topic: "new topic".into(),
			..before.clone()
		};
		run(
			&mut state,
			Id(20),
			Action::Edit {
				before,
				after: after.clone(),
			},
			&mut next_id,
		);
		run(&mut state, Id(20), Action::Load, &mut next_id);
		assert_eq!(state.channel_details(Id(20)), Some(&after));
		run(
			&mut state,
			Id(20),
			Action::CreateCategory {
				name: "spaces".into(),
			},
			&mut next_id,
		);
		let category = Id(next_id);
		assert_eq!(state.channel(category).unwrap().kind, 4);
		assert_eq!(state.channel(category).unwrap().parent_id, None);
		use client_core::channel_actions::CreateKind;
		for kind in [CreateKind::Text, CreateKind::Voice, CreateKind::Forum] {
			run(
				&mut state,
				category,
				Action::Create {
					name: "new-channel".into(),
					kind,
				},
				&mut next_id,
			);
			assert!(state.can_view(Id(next_id)));
			assert_eq!(state.channel(Id(next_id)).unwrap().kind, kind.wire_kind());
			assert_eq!(
				state.channel(Id(next_id)).unwrap().parent_id,
				Some(category)
			);
		}
		let created = Id(next_id);
		run(&mut state, created, Action::Delete, &mut next_id);
		assert!(state.channel(created).is_none());
	}
}
