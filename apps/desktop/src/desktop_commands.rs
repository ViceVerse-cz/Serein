//! Route explicit UI commands to their existing owners.
use super::{
	AuthState, Command, Desktop, Failure, attachment_send_allowed, interaction_uploads, uploads,
};
#[cfg(feature = "demo")]
use super::{
	Envelope, Event, channel_demo, community_admin_demo, community_demo, components_demo,
	demo_members, dm_demo, onboarding_demo, polls_demo, profile_settings_demo,
	server_settings_demo, slash_demo,
};

impl Desktop {
	/// Dispatches one queued command to the demo or live transport.
	pub(super) fn command(&mut self, command: Command) {
		if let Command::MemberSearchLocal(request) = &command {
			self.queue_member_cache_search(request);
			return;
		}
		if let Command::MemberSearch(request) = &command {
			self.queue_member_cache_search(request);
		}
		if let Some(connection) = &self.connection {
			connection.sync_admin_reads(&self.state);
		}
		if let Command::Report(request) = &command
			&& !self.state.report_command_allowed(request)
		{
			self.state.command_rejected(command);
			return;
		}
		if let Command::Soundboard(request) = &command
			&& !self.state.soundboard_command_allowed(request)
		{
			self.state.command_rejected(command);
			return;
		}
		if !self.state.gateway_connected
			&& self.state.auth == AuthState::Authenticated
			&& matches!(command, Command::Send { .. })
			&& let Some(connection) = &self.connection
		{
			connection.recover_send();
		}
		if let Command::Autocomplete(request) = &command
			&& !self.state.autocomplete_dispatch_allowed(request)
		{
			self.state.command_rejected(command);
			return;
		}

		if let Command::Interaction(request) = &command
			&& !self.state.application_command_dispatch_allowed(request)
		{
			self.state.command_rejected(command);
			return;
		}
		if matches!(&command, Command::Interaction(request) if interaction_uploads::request_has_files(request))
		{
			self.interaction_upload(command);
			return;
		}

		if let Command::Community {
			guild,
			request,
			action,
			..
		} = &command
			&& !self.state.demo
			&& !self
				.state
				.community_command_allowed(*guild, *request, action)
		{
			self.state.command_rejected(command);
			return;
		}
		if let Command::InboxAction { request, action } = &command
			&& (self.state.inbox.request != *request
				|| self.state.inbox.operation.as_ref() != Some(action)
				|| !self.state.inbox_action_allowed(action))
		{
			self.state.command_rejected(command);
			return;
		}
		if let Command::ChannelPreferences(request) = &command
			&& !self.state.channel_preference_request_allowed(request)
		{
			self.state.command_rejected(command);
			return;
		}
		if let Command::ChannelAction {
			guild,
			channel,
			request,
			action,
		} = &command
			&& matches!(action, client_core::channel_actions::Action::Personal(_))
			&& !self
				.state
				.personal_channel_dispatch_allowed(*guild, *channel, *request, action)
		{
			self.state.command_rejected(command);
			return;
		}
		if let Command::Mfa(request) = &command
			&& !self.state.mfa_command_allowed(request)
		{
			self.state.command_rejected(command);
			return;
		}
		if let Command::ServerAdmin {
			guild,
			request,
			action,
		} = &command
			&& !self
				.state
				.server_admin_command_allowed(*guild, *request, action)
		{
			self.state.command_rejected(command);
			return;
		}
		if let Command::MemberGuide {
			guild,
			user,
			request,
			..
		} = &command
			&& !self
				.state
				.member_guide_command_allowed(*guild, *user, *request)
		{
			self.state.command_rejected(command);
			return;
		}
		if let Command::Onboarding { guild, request, .. } = &command
			&& !self.state.onboarding_command_allowed(*guild, *request)
		{
			self.state.command_rejected(command);
			return;
		}
		if let Command::ServerSettings {
			guild,
			request,
			edit,
		} = &command
			&& !self
				.state
				.server_settings_command_allowed(*guild, *request, edit)
		{
			self.state.command_rejected(command);
			return;
		}
		if let Command::EditProfile {
			user,
			request,
			changes,
		} = &command
			&& !self
				.state
				.own_profile_command_allowed(*user, *request, changes)
		{
			self.state.command_rejected(command);
			return;
		}
		if let Command::ServerProfile {
			user,
			guild,
			request,
			edit,
		} = &command
			&& !self
				.state
				.server_profile_command_allowed(*user, *guild, *request, edit)
		{
			self.state.command_rejected(command);
			return;
		}
		if let Command::AccountConnections {
			user,
			request,
			change,
		} = &command
			&& !self
				.state
				.account_connections_command_allowed(*user, *request, change)
		{
			self.state.command_rejected(command);
			return;
		}
		if let Command::OwnedCosmetics {
			user,
			guild,
			request,
			change,
			nitro,
		} = &command
			&& !self
				.state
				.owned_cosmetics_command_allowed(*user, *guild, *request, *change, *nitro)
		{
			self.state.command_rejected(command);
			return;
		}
		if let Command::Send { channel, nonce, .. } = &command
			&& self
				.state
				.pending
				.iter()
				.any(|p| p.nonce == *nonce && !p.attachments.is_empty())
		{
			let channel = *channel;
			let available = attachment_send_allowed(&self.state, channel)
				&& !self.fixture_only
				&& self.connection.is_some();
			if available
				&& let Some(source) = self.uploads.take_source(self.state.generation, channel)
			{
				let (progress, receive) =
					tokio::sync::watch::channel(discord_api::upload::Status::Preparing);
				let (cancel, _) = tokio::sync::watch::channel(false);
				if self.uploads.begin_upload(receive, cancel.clone()).is_ok() {
					let request = uploads::UploadRequest {
						command,
						source,
						voice_message: None,
						progress,
						cancel,
					};
					self.messaging.attachment = None;
					if let Err(error) = self.connection.as_ref().unwrap().uploads.try_send(request)
					{
						let request = error.into_inner();
						request
							.progress
							.send_replace(discord_api::upload::Status::Failed(
								"Upload queue full; reselect the file",
							));
						self.state.command_rejected(request.command);
					}
					return;
				}
			}
			// Nothing left the client: keep the caption, reply and staged files for a retry.
			let failure = "File not sent; check the connection and attachment, then send again";
			if self.state.return_unsent(command, failure) {
				self.messaging.draft_changes.push(channel);
			}
			self.messaging
				.toasts
				.push(ui::design::Level::Error, failure);
			return;
		}
		// A forum post with files takes the same staged-upload path as a message.
		if let Command::CreatePost {
			parent,
			attachments,
			request,
			..
		} = &command
			&& !attachments.is_empty()
		{
			let (parent, request) = (*parent, *request);
			let available = !self.state.demo
				&& self.state.can_attach_post(parent)
				&& !self.fixture_only
				&& self.state.auth == AuthState::Authenticated
				&& self.state.gateway_connected
				&& self.state.selected == Some(parent)
				&& self.connection.is_some();
			if available
				&& let Some(source) = self.uploads.take_source(self.state.generation, parent)
			{
				let (progress, receive) =
					tokio::sync::watch::channel(discord_api::upload::Status::Preparing);
				let (cancel, _) = tokio::sync::watch::channel(false);
				if self.uploads.begin_upload(receive, cancel.clone()).is_ok() {
					let request = uploads::UploadRequest {
						command,
						source,
						voice_message: None,
						progress,
						cancel,
					};
					self.messaging.attachment = None;
					if let Err(error) = self.connection.as_ref().unwrap().uploads.try_send(request)
					{
						let request = error.into_inner();
						request
							.progress
							.send_replace(discord_api::upload::Status::Failed(
								"Upload queue full; reselect the file",
							));
						self.state.command_rejected(request.command);
					}
					return;
				}
			}
			self.state.apply_post(
				parent,
				request,
				Err(Failure::ProtocolAt(
					"Post not created; reconnect and reselect the attachment",
				)),
			);
			return;
		}
		if let Command::History {
			channel,
			before: None,
			after: None,
			..
		} = &command
			&& !self.fixture_only
			&& self.state.selected == Some(*channel)
			&& self.state.can_call(*channel)
			&& self
				.state
				.channels
				.iter()
				.any(|c| c.id == *channel && c.guild.is_none())
		{
			self.command(Command::Voice(client_core::voice::Command::Sync {
				channel: *channel,
			}));
		}
		if let Command::Voice(control) = &command {
			if self.state.demo || self.fixture_only {
				self.state.status = "Voice calls are unavailable in the offline preview";
				return;
			}
			self.voice_messages.cancel();
			self.messaging.voice_messages.close();
			self.messaging.voice_messages.capture_busy = self.voice_messages.capture_busy();
			if let client_core::voice::Command::Join {
				channel,
				request,
				ring,
				..
			} = control
			{
				let result =
					self.voice
						.begin_with_video(&self.state, *ring, self.messaging.video_settings);
				if let Err(message) = result {
					self.state.apply_voice(client_core::voice::Event::Failed {
						channel: *channel,
						request: *request,
						message,
					});
					return;
				}
			}
			if matches!(
				control,
				client_core::voice::Command::SetCamera { enabled: false, .. }
			) {
				self.voice.stop_camera();
				self.messaging.voice_camera_preview = None;
			}
			if matches!(control, client_core::voice::Command::Leave { .. }) {
				self.voice_messages.cancel();
				self.voice.stop();
				self.messaging.camera_test_requested = false;
				self.messaging.camera_test_texture = None;
			}
		}
		if matches!(
			&command,
			Command::History { .. } | Command::HistoryAround { .. }
		) {
			self.load_cached_history();
		}
		#[cfg(feature = "demo")]
		if self.state.demo {
			let event = match command {
				// Offline previews confirm locally; nothing reaches the service.
				Command::BadgeSettings { request, settings } => Event::BadgeSettings {
					request,
					result: Ok(settings),
				},
				Command::ApplicationCommands {
					channel,
					guild,
					request,
					..
				} => Event::ApplicationCommands {
					channel,
					request,
					result: Ok(slash_demo::catalog(guild)),
				},
				Command::Autocomplete(request) => slash_demo::autocomplete(request),
				Command::Interaction(request) => {
					if matches!(
						&request.data,
						client_core::interactions::Data::ApplicationCommand { .. }
					) {
						slash_demo::respond(&mut self.state, request);
						return;
					}
					if std::env::args().any(|arg| arg == "--demo-components") {
						self.state.apply(Envelope {
							generation: self.state.generation,
							event: components_demo::respond(request),
						});
						return;
					}
					Event::Interaction(client_core::interactions::Event::Submitted {
						nonce: request.nonce,
						result: Err(Failure::ProtocolAt(
							"Offline preview does not contact applications",
						)),
					})
				}
				Command::MemberSearchLocal(_) => return,
				Command::MemberSearch(request) => {
					let query = request.query.to_lowercase();
					let rows = demo_members(
						&self.state,
						Some(request.guild),
						request.channel,
						request.nonce,
					)
					.slots
					.into_iter()
					.flatten()
					.filter_map(|slot| match slot {
						model::MemberSlot::Person(member) => Some(member),
						_ => None,
					})
					.filter(|member| {
						member.user.name.to_lowercase().contains(&query)
							|| member
								.nick
								.as_ref()
								.is_some_and(|name| name.to_lowercase().contains(&query))
							|| member.user.id.to_string() == query
					})
					.collect();
					Event::MemberSearch {
						request,
						result: Ok(rows),
					}
				}

				// Demo preference changes are applied synchronously by client-core.
				Command::Mfa(_) => return,
				Command::MessagingPermissions { .. } => return,
				Command::AccountOverview { .. } | Command::AccountConnections { .. } => return,
				Command::ServerProfile {
					user,
					guild,
					request,
					edit,
				} => profile_settings_demo::execute(&self.state, user, guild, request, edit),
				Command::OwnedCosmetics {
					user,
					guild,
					request,
					change,
					..
				} => profile_settings_demo::owned_cosmetics(
					&self.state,
					user,
					guild,
					request,
					change,
				),
				Command::ChannelPreferences(request) => {
					Event::ChannelPreferences(self.state.demo_channel_preferences(&request))
				}
				Command::ChannelAction {
					guild,
					channel,
					request,
					action,
				} => channel_demo::execute(
					&self.state,
					guild,
					channel,
					request,
					action,
					&mut self.synthetic_id,
				),
				Command::Community {
					guild,
					request,
					action,
					..
				} => community_demo::execute(&mut self.state, guild, request, *action),
				Command::ServerAdmin {
					guild,
					request,
					action,
				} => server_settings_demo::execute_admin(&self.state, guild, request, *action),
				Command::ServerSettings {
					guild,
					request,
					edit,
				} => server_settings_demo::execute(&self.state, guild, request, edit),
				Command::MemberGuide {
					guild,
					user,
					request,
					action,
				} => community_admin_demo::execute_member(&self.state, guild, user, request, action),
				Command::Onboarding {
					guild,
					request,
					action,
				} => onboarding_demo::execute(guild, request, action),
				Command::GuildFolders(settings) => Event::GuildFolders(Ok(settings
					.map(|(_, settings)| settings)
					.unwrap_or_default())),
				Command::SendServerInvite {
					guild,
					user,
					code,
					nonce,
					request,
				} => {
					let channel = dm_demo::channel(&self.state, user).unwrap();
					self.synthetic_id += 1;
					let mut message = test_support::message(self.synthetic_id, channel.id);
					message.author = self.state.user.clone().unwrap();
					message.content = format!("https://discord.gg/{code}");
					message.nonce = Some(nonce);
					Event::ServerAction(client_core::server_actions::Event::InviteSent {
						guild,
						user,
						request,
						result: Ok(Box::new((channel, message))),
					})
				}
				Command::ServerAction { action, request } => {
					server_settings_demo::execute_action(&mut self.state, action, request)
				}
				Command::UserAction {
					action: client_core::user_actions::Action::OpenDm(user),
					request,
					..
				} => Event::UserAction(client_core::user_actions::Event::DmOpened {
					user,
					request,
					result: dm_demo::channel(&self.state, user).map(Box::new),
				}),
				Command::UserAction {
					action: client_core::user_actions::Action::LoadNote(user),
					request,
					..
				} => Event::UserAction(client_core::user_actions::Event::NoteLoaded {
					user,
					request,
					result: Ok(self.state.user_note(user).unwrap_or("").to_owned()),
				}),
				Command::UserAction {
					action, request, ..
				} => Event::UserAction(client_core::user_actions::Event::Written {
					action,
					request,
					result: Ok(()),
				}),
				Command::GroupAction { action, request } => Event::GroupAction(
					self.demo_groups
						.respond(&self.state, action, request, &mut self.synthetic_id),
				),
				Command::MarkRead {
					channel,
					message,
					request,
					..
				} => Event::ReadState(client_core::read_state::Event::Result {
					channel,
					message,
					request,
					result: Ok(()),
				}),
				Command::MarkGuildRead { guild, request } => {
					Event::ReadState(client_core::read_state::Event::GuildAck {
						guild,
						request,
						result: Ok(()),
					})
				}
				Command::MarkAllRead { request, channels } => {
					Event::ReadState(client_core::read_state::Event::AllAck {
						request,
						acked: channels.len(),
						result: Ok(()),
					})
				}
				Command::Polls(request) => {
					polls_demo::respond(&self.state, request, &mut self.synthetic_id)
				}
				// The offline preview answers a play request without any audio or network use.
				Command::Soundboard(client_core::soundboard::Request::Send {
					request,
					channel,
					sound,
					..
				}) => Event::Soundboard(client_core::soundboard::Event::Sent {
					request,
					channel,
					sound,
					result: Ok(()),
				}),
				Command::Soundboard(_) => return,
				Command::Reactions(command) => {
					use client_core::reactions::{Command as R, Event as E};
					Event::Reactions(match command {
						R::Read {
							channel,
							message,
							request,
						} => E::Read {
							channel,
							message,
							request,
							result: Ok(vec![]),
						},
						R::Set {
							channel,
							message,
							emoji,
							add,
							request,
						} => {
							let mut reactions = self
								.state
								.timeline
								.get(message)
								.and_then(|m| m.reactions.clone())
								.unwrap_or_default();
							if let Some(r) = reactions.iter_mut().find(|r| r.emoji.same(&emoji)) {
								if r.me != add {
									r.count = if add {
										r.count + 1
									} else {
										r.count.saturating_sub(1)
									};
									r.me = add;
								}
							} else if add {
								reactions.push(model::Reaction {
									emoji,
									count: 1,
									me: true,
									me_burst: false,
								});
							}
							reactions.retain(|r| r.count > 0);
							self.state.reactions.reset();
							let _ = self.state.timeline.set_reactions(message, Some(reactions));
							// The fixture has no service readback; it updates synthetic RAM only.
							E::Written {
								channel,
								message,
								request,
								result: Ok(()),
							}
						}
						R::Users {
							channel,
							message,
							emoji,
							request,
							..
						} => {
							let users = self
								.state
								.timeline
								.get(message)
								.map(|message| vec![message.author.clone()])
								.unwrap_or_default();
							E::Users {
								channel,
								message,
								emoji,
								request,
								result: Ok(users),
							}
						}
					})
				}
				Command::Voice(_)
				| Command::CancelProfile
				| Command::CancelSearch
				| Command::CancelAutocomplete
				| Command::CancelMentions
				| Command::Prefetch(_) => return,
				Command::Report(request) => test_support::report_demo(*request),
				command @ (Command::InboxAction { .. } | Command::Mentions { .. }) => {
					self.inbox_demo.execute(&self.state, command)
				}
				Command::ThreadStarter {
					thread,
					parent,
					request,
				} => {
					// The fixture thread hangs off a synthetic parent message with its own id.
					let mut message = test_support::message(thread.0, parent);
					message.content =
						"This synthetic message started the thread; replies continue below.".into();
					Event::ThreadStarter {
						thread,
						request,
						result: Ok(message),
					}
				}
				Command::CreatePost {
					parent,
					guild,
					title,
					tags,
					request,
					..
				} => {
					self.synthetic_id += 1;
					Event::PostCreated {
						parent,
						request,
						result: Ok(model::Channel {
							id: model::Id(self.synthetic_id),
							guild: Some(guild),
							parent_id: Some(parent),
							position: 0,
							name: title,
							icon: None,
							kind: 11,
							recipients: vec![],
							last_message: None,
							member_list_id: None,
							tags: (!tags.is_empty()).then(|| {
								Box::new(model::forum::Tags {
									applied: tags,
									..Default::default()
								})
							}),
							message_count: Some(0),
						}),
					}
				}
				Command::Archives {
					parent,
					guild,
					kind,
					before,
					request,
				} => {
					use model::archives::{Cursor, Kind, Page};
					let offset = parent.0.saturating_mul(10_000).saturating_add(match kind {
						Kind::Public => 0,
						Kind::Private => 1_000,
						Kind::JoinedPrivate => 2_000,
					});
					let ids = (if before.is_none() {
						[900, 850, 800]
					} else {
						[700, 650, 600]
					})
					.map(|id| offset.saturating_add(id));
					let public_kind = if self
						.state
						.channels
						.iter()
						.any(|c| c.id == parent && c.kind == 5)
					{
						10
					} else {
						11
					};
					let threads = ids
						.into_iter()
						.map(|id| model::Channel {
							id: model::Id(id),
							guild: Some(guild),
							parent_id: Some(parent),
							position: 0,
							name: format!("Synthetic archived thread {id}"),
							icon: None,
							kind: if kind == Kind::Public {
								public_kind
							} else {
								12
							},
							recipients: vec![],
							last_message: None,
							member_list_id: None,
							tags: None,
							message_count: None,
						})
						.collect();
					Event::Archives {
						parent,
						request,
						result: Ok(Page {
							threads,
							next: before.is_none().then_some(if kind == Kind::JoinedPrivate {
								Cursor::Id(model::Id(ids[2]))
							} else {
								Cursor::Time(1_788_998_400_000_000_000)
							}),
						}),
					}
				}
				Command::Pins {
					channel,
					before,
					request,
				} => {
					// Explicit synthetic pins, independent of message creation order.
					let hits = if before.is_none() {
						[480, 499, 470]
					} else {
						[420, 455, 430]
					}
					.into_iter()
					.map(|id| {
						let message = test_support::message(id, channel);
						model::SearchHit {
							id: message.id,
							channel,
							excerpt: format!(
								"Synthetic pinned message: {}",
								message.content.chars().take(200).collect::<String>()
							),
							author: message.author,
							mentions: message.mentions,
							attachments: message.attachments,
							embeds: message.embeds,
						}
					})
					.collect();
					Event::Search {
						channel,
						request,
						result: Ok(client_core::search::Outcome::Pins(model::SearchPage {
							hits,
							total: 0,
							partial: before.is_none(),
							pin_cursor: before.is_none().then_some(1_788_998_400_000_000_000),
						})),
					}
				}
				Command::Search {
					channel,
					query,
					before,
					offset,
					oldest,
					request,
					..
				} => {
					let mut hits = Vec::new();
					let mut total = 0;
					let (content, filters) = match model::search_terms(&query) {
						Ok(terms) => terms,
						Err(_) => return,
					};
					// Fixture IDs repeat per channel, so an offline search reads one channel.
					let source = filters
						.iter()
						.find_map(|(key, value)| (*key == "channel_id").then(|| value.parse().ok()))
						.flatten()
						.map_or(channel, model::Id);
					for id in (1..=500)
						.rev()
						// The cursor continues older newest first, newer oldest first.
						.filter(|id| {
							before.is_none_or(|b| if oldest { *id > b.0 } else { *id < b.0 })
						}) {
						let message = test_support::message(id, source);
						if message
							.content
							.to_lowercase()
							.contains(&content.to_lowercase())
							&& filters.iter().all(|(group, _)| {
								filters.iter().filter(|(key, _)| key == group).any(
									|(key, value)| match *key {
										"author_id" => message.author.id.to_string() == *value,
										"channel_id" => source.to_string() == *value,
										"mentions" => message
											.mentions
											.iter()
											.any(|user| user.id.to_string() == *value),
										"min_id" => {
											value.parse::<u64>().is_ok_and(|min| message.id.0 > min)
										}
										"max_id" => {
											value.parse::<u64>().is_ok_and(|max| message.id.0 < max)
										}
										"pinned" => {
											self.state.is_pinned(source, message.id)
												== (value == "true")
										}
										"author_type" => match value.as_str() {
											"webhook" => message.author.webhook,
											"user" => !message.author.webhook,
											_ => false, // The offline fixture contains no bot authors.
										},
										"has" => match value.as_str() {
											"link" => {
												message.content.contains("https://")
													|| message.content.contains("http://")
											}
											"embed" => !message.embeds.is_empty(),
											"file" => !message.attachments.is_empty(),
											"image" => message.attachments.iter().any(|a| {
												a.content_type
													.as_deref()
													.is_some_and(|mime| mime.starts_with("image/"))
											}),
											"video" => {
												message.attachments.iter().any(|a| a.is_video())
											}
											"sound" => {
												message.attachments.iter().any(|a| a.is_audio())
											}
											_ => false,
										},
										_ => false,
									},
								)
							}) {
							total += 1;
							// The fixture is small: keep every match, then page in the chosen order.
							{
								hits.push(model::SearchHit {
									id: message.id,
									channel: source,
									author: message.author,
									mentions: message.mentions,
									excerpt: message.content.clone(),
									attachments: message.attachments,
									embeds: message.embeds,
								});
							}
						}
					}
					hits.sort_unstable_by_key(|hit| std::cmp::Reverse(hit.id));
					if oldest {
						hits.reverse();
					}
					let hits = hits
						.into_iter()
						.skip(offset as usize)
						.take(model::SEARCH_PAGE_SIZE)
						.collect();
					Event::Search {
						channel,
						request,
						result: Ok(client_core::search::Outcome::Page(model::SearchPage {
							hits,
							total,
							partial: false,
							pin_cursor: None,
						})),
					}
				}
				Command::Gifs { query, request } => Event::Gifs {
					request,
					result: Ok(test_support::gif_page(query.as_deref())),
				},
				Command::CancelGifs => return,
				Command::GifFavorites { request, .. } => Event::GifFavorites {
					request,
					result: Ok(self.state.gifs.favorites.clone()),
				},
				Command::ReactionPreferences { request } => Event::ReactionPreferences {
					request,
					result: Ok(self.state.reaction_preferences.ranked.clone()),
				},
				Command::CreateGuild { sequence, .. } => Event::GuildCreated {
					sequence,
					result: Err(Failure::ProtocolAt("Server creation unavailable offline")),
				},
				Command::JoinInvite { request, .. } => Event::JoinInvite {
					request,
					result: Err(Failure::ProtocolAt("Server joining unavailable offline")),
				},
				Command::Invite { code } => Event::Invite {
					code,
					result: Err(Failure::Protocol),
				},
				Command::Profile {
					user,
					guild,
					request,
					..
				} => Event::Profile {
					user,
					guild,
					request,
					result: Err(Failure::Protocol),
				},
				Command::StreamPreview {
					guild,
					channel,
					user,
					request,
				} => Event::StreamPreview {
					guild,
					channel,
					user,
					request,
					result: Ok(format!(
						"https://cdn.discordapp.com/streams/guild:{guild}:{channel}:{user}/0123456789abcdef.png"
					)),
				},
				Command::EditProfile {
					user,
					request,
					changes,
				} => profile_settings_demo::global_profile(&self.state, user, request, changes),
				Command::Members {
					guild,
					channel,
					request,
					..
				} => {
					let Some(channel) = channel else {
						return;
					};
					Event::Members(demo_members(&self.state, guild, channel, request))
				}
				Command::ForumPosts { .. } | Command::ForumSummaries { .. } => return,
				command @ (Command::History { .. }
				| Command::HistoryAround { .. }
				| Command::ReplyPreview { .. }
				| Command::CancelReplyPreviews) => {
					if let Some(event) = test_support::offline_history_event(
						&self.state,
						&command,
						std::env::args().any(|arg| arg == "--demo-message-links"),
					) {
						self.state.apply(Envelope {
							generation: self.state.generation,
							event,
						});
					}
					return;
				}
				Command::StickerPacks => Event::StickerPacks(Ok(self.state.stickers.packs.clone())),
				Command::Sticker(id) => Event::Sticker {
					id,
					result: self
						.state
						.guilds
						.iter()
						.filter_map(|g| g.stickers.as_ref())
						.flatten()
						.chain(
							self.state
								.stickers
								.packs
								.iter()
								.flat_map(|p| p.stickers.iter()),
						)
						.find(|s| s.id == id)
						.cloned()
						.ok_or(Failure::Protocol),
				},
				// The offline fixture never issues challenges, so it never resumes one.
				Command::VerifiedSend { nonce, .. } => Event::SendResult {
					nonce,
					result: Err(Failure::Protocol),
				},
				Command::Forward {
					message,
					channel,
					nonce,
					..
				} => {
					self.synthetic_id += 1;
					let result = self
						.state
						.timeline
						.get(message)
						.cloned()
						.ok_or(Failure::Protocol)
						.map(|mut m| {
							m.id = model::Id(self.synthetic_id);
							m.channel = channel;
							m.author = self.state.user.clone().unwrap();
							m.author_nick = None;
							m.author_roles.clear();
							m.nonce = Some(nonce.clone());
							m.reply_to = None;
							m.reply_deleted = false;
							m.kind = 0;
							m.forwarded = true;
							m.reactions = None;
							m
						});
					Event::SendResult { nonce, result }
				}
				Command::Send {
					sticker,
					channel,
					content,
					nonce,
					reply,
				} => {
					self.synthetic_id += 1;
					let mut message = test_support::message(self.synthetic_id, channel);
					message.author = self.state.user.clone().unwrap();
					message.content = content;
					message.sticker_items = sticker
						.and_then(|id| {
							self.state
								.pending
								.iter()
								.find_map(|p| p.sticker.as_ref().filter(|s| s.id == id).cloned())
						})
						.into_iter()
						.collect();
					message.nonce = Some(nonce.clone());
					message.reply_to = reply.map(client_core::Reply::target);
					Event::SendResult {
						nonce,
						result: Ok(message),
					}
				}
				Command::Edit {
					request,
					channel,
					message,
					content,
				} => {
					let result = self
						.state
						.timeline
						.get(message)
						.cloned()
						.ok_or(Failure::Protocol)
						.map(|mut updated| {
							updated.content = content;
							updated.edited = true;
							updated.edited_at =
								Some(updated.edited_at.unwrap_or(0).saturating_add(1));
							updated
						});
					Event::Edited {
						request,
						channel,
						message,
						result,
					}
				}
				Command::Delete { channel, message } => Event::Delete {
					channel,
					id: message,
				},
				Command::Pin {
					request,
					channel,
					message,
					pinned,
				} => Event::Pinned {
					request,
					channel,
					message,
					pinned,
					result: Ok(()),
				},
			};
			self.state.apply(Envelope {
				generation: self.state.generation,
				event,
			});
			self.state.status = "Offline fixture · action affected synthetic RAM only";
			return;
		}
		if let Some(connection) = &self.connection {
			if let Err(error) = connection.commands.try_send(command) {
				self.state.command_rejected(error.into_inner());
			}
		} else {
			self.state.command_rejected(command);
		}
	}
}
