//! Exhaustive command dispatch under the existing read and MFA scopes.
use super::{
	ADMIN_READ, Command, DiscordApi, Duration, Event, Failure, Id, Method, PROFILE_EXTRAS,
	SPECULATIVE, allowed_mentions, mfa, reaction_path, reaction_users_path, stream_preview_url,
};
use discord_protocol::{MessageDto, UserDto, decode, profile};

impl DiscordApi {
	/// Runs one typed command and returns its typed event.
	/// [`Self::execute`] for the write queue. Like the official client, a message Discord
	/// rate-limits (documented: a 429 request is not processed, so nothing was created) stays
	/// pending and is resent with the same nonce once the service's wait has passed; `execute`
	/// sleeps out that shared cooldown first. `throttled` hears each wait. Ambiguous outcomes are
	/// never resent.
	pub async fn execute_write(&self, command: Command, mut throttled: impl FnMut(&str)) -> Event {
		/// Resends of one message, each after the wait Discord asked for.
		const MAX_SEND_RETRIES: usize = 30;
		let resend = match &command {
			Command::Send {
				sticker,
				channel,
				content,
				nonce,
				reply,
			} => Some((*sticker, *channel, content.clone(), nonce.clone(), *reply)),
			_ => None,
		};
		let mut event = self.execute(command).await;
		let Some((sticker, channel, content, nonce, reply)) = resend else {
			return event;
		};
		for _ in 0..MAX_SEND_RETRIES {
			if !matches!(
				event,
				Event::SendResult {
					result: Err(Failure::RateLimited),
					..
				}
			) || self.stopped()
			{
				break;
			}
			throttled(&nonce);
			event = self
				.execute(Command::Send {
					sticker,
					channel,
					content: content.clone(),
					nonce: nonce.clone(),
					reply,
				})
				.await;
		}
		event
	}
	pub async fn execute(&self, command: Command) -> Event {
		if client_core::server_admin::ReadKey::of(&command, 0).is_some() {
			ADMIN_READ.scope((), self.execute_scoped(command)).await
		} else {
			self.execute_scoped(command).await
		}
	}
	pub(super) async fn execute_scoped(&self, command: Command) -> Event {
		if let Some(target) = client_core::mfa::Target::from_command(&command) {
			mfa::OPERATION
				.scope(target, self.execute_unscoped(command))
				.await
		} else {
			self.execute_unscoped(command).await
		}
	}
	pub(super) async fn execute_unscoped(&self, command: Command) -> Event {
		match command {
			Command::ChannelPreferences(request) => {
				Event::ChannelPreferences(client_core::preferences_sync::Finished {
					generation: request.generation,
					guild: request.guild,
					request: request.request,
					result: self.channel_preferences(&request).await,
				})
			}
			Command::Mfa(request) => Event::Mfa(self.mfa_request(*request).await),
			Command::Report(request) => Event::Report(client_core::reporting::Event {
				target: request.target,
				request: request.request,
				result: self.report(&request).await,
			}),
			Command::CancelAutocomplete => Event::Autocomplete(
				client_core::application_commands::CompletionEvent::Submitted {
					nonce: String::new(),
					result: Ok(()),
				},
			),
			Command::Autocomplete(request) => Event::Autocomplete(
				client_core::application_commands::CompletionEvent::Submitted {
					result: self.interaction(&request, None).await,
					nonce: request.nonce,
				},
			),

			Command::InboxAction { request, action } => Event::InboxAction {
				request,
				result: self.inbox_action(&action).await,
			},
			Command::Mentions { before, request } => Event::Mentions {
				request,
				result: self.mentions(before).await,
			},
			Command::ApplicationCommands {
				channel,
				guild,
				context,
				request,
			} => Event::ApplicationCommands {
				channel,
				request,
				result: self.application_commands(channel, guild, context).await,
			},
			Command::Interaction(request) => {
				Event::Interaction(client_core::interactions::Event::Submitted {
					result: self.interaction(&request, None).await,
					nonce: request.nonce,
				})
			}
			Command::MessagingPermissions { request, change } => Event::MessagingPermissions {
				request,
				result: self.account_messaging_permissions(change).await,
			},
			Command::Community {
				guild,
				user,
				request,
				action,
				..
			} => Event::Community(self.community(guild, user, request, &action).await),
			Command::ServerAdmin {
				guild,
				request,
				action,
			} => Event::ServerAdmin(self.server_admin(guild, request, &action).await),
			Command::ServerSettings {
				guild,
				request,
				edit,
			} => Event::ServerSettings(self.server_settings(guild, request, edit).await),
			Command::MemberGuide {
				guild,
				user,
				request,
				action,
			} => Event::MemberGuide(self.member_guide(guild, user, request, action).await),
			Command::Onboarding {
				guild,
				request,
				action,
			} => Event::Onboarding(self.onboarding(guild, request, action).await),
			Command::SendServerInvite {
				guild,
				user,
				code,
				nonce,
				request,
			} => Event::ServerAction(client_core::server_actions::Event::InviteSent {
				guild,
				user,
				request,
				result: self
					.send_server_invite(user, &code, &nonce)
					.await
					.map(Box::new),
			}),
			Command::GroupAction { action, request } => {
				if let client_core::group_actions::Action::Create(users) = action {
					return Event::GroupAction(client_core::group_actions::Event::Created {
						request,
						result: self.create_group(users).await,
					});
				}
				if let client_core::group_actions::Action::TransferOwner { channel, user } = action
				{
					return Event::GroupAction(client_core::group_actions::Event::MembersLoaded {
						channel,
						request,
						result: self.transfer_group_owner(channel, user).await,
					});
				}
				if let client_core::group_actions::Action::LoadMembers(channel) = action {
					return Event::GroupAction(client_core::group_actions::Event::MembersLoaded {
						channel,
						request,
						result: self.group_members(channel).await,
					});
				}
				Event::GroupAction(client_core::group_actions::Event::Written {
					channel: action.channel(),
					request,
					result: self.group_action(action).await,
				})
			}
			Command::JoinInvite {
				code,
				request,
				captcha,
			} => self.join_invite(&code, request, captcha).await,
			Command::CreateGuild { request, sequence } => Event::GuildCreated {
				sequence,
				result: self.create_guild(&request).await,
			},
			Command::GuildFolders(settings) => Event::GuildFolders(match settings {
				Some((base, settings)) => self.save_guild_folders(base, settings).await,
				None => self.guild_folders().await,
			}),
			Command::ChannelAction {
				guild,
				channel,
				request,
				action,
			} => Event::ChannelAction(client_core::channel_actions::Event::Finished {
				guild,
				channel,
				request,
				result: self.channel_action(guild, channel, &action).await,
			}),
			Command::ServerAction { action, request } => {
				Event::ServerAction(client_core::server_actions::Event::Written {
					action,
					request,
					result: self.server_action(action).await,
				})
			}
			Command::UserAction {
				action,
				request,
				captcha,
			} => {
				if let client_core::user_actions::Action::OpenDm(user) = action {
					let mut challenge = None;
					let result = self
						.open_dm(user, captcha.as_deref(), Some(&mut challenge))
						.await;
					if let Some(challenge) = challenge {
						return Event::UserAction(client_core::user_actions::Event::Challenge {
							action,
							request,
							challenge: Box::new(challenge),
						});
					}
					return Event::UserAction(client_core::user_actions::Event::DmOpened {
						user,
						request,
						result: result.map(Box::new),
					});
				}
				if let client_core::user_actions::Action::LoadNote(user) = action {
					return Event::UserAction(client_core::user_actions::Event::NoteLoaded {
						user,
						request,
						result: self.user_note(user).await,
					});
				}
				let mut challenge = None;
				let slot =
					client_core::user_actions::challengeable(&action).then_some(&mut challenge);
				let result = self.user_action(&action, captcha.as_deref(), slot).await;
				if let Some(challenge) = challenge {
					return Event::UserAction(client_core::user_actions::Event::Challenge {
						action,
						request,
						challenge: Box::new(challenge),
					});
				}
				Event::UserAction(client_core::user_actions::Event::Written {
					action,
					request,
					result,
				})
			}
			Command::Invite { code } => {
				let result = self.invite(&code).await.map(Box::new);
				Event::Invite { code, result }
			}
			Command::CreatePost {
				parent,
				guild,
				title,
				content,
				attachments,
				tags,
				request,
			} => {
				// Files are staged by the upload worker, which owns the whole post request.
				let result = if attachments.is_empty() {
					self.create_post(parent, guild, (&title, &tags), &content, None)
						.await
				} else {
					Err(Failure::ProtocolAt(
						"Upload unavailable; reselect the file to retry",
					))
				};
				Event::PostCreated {
					parent,
					request,
					result,
				}
			}
			Command::ForumSummaries { channels, request } => {
				Event::ForumSummaries {
					request,
					results: futures_util::future::join_all(channels.into_iter().map(
						|channel| async move { (channel, self.forum_summary(channel).await) },
					))
					.await,
				}
			}
			Command::ForumPosts {
				parent,
				guild,
				offset,
				request,
			} => {
				let result = self.forum_posts(parent, guild, offset).await;
				Event::ForumPosts {
					parent,
					request,
					result,
				}
			}
			Command::Archives {
				parent,
				guild,
				kind,
				before,
				request,
			} => {
				let result = self.archives(parent, guild, kind, before).await;
				Event::Archives {
					parent,
					request,
					result,
				}
			}
			Command::ThreadStarter {
				thread,
				parent,
				request,
			} => {
				// Documented single-message read; a thread shares its id with its starter.
				let result = self
					.request(
						Method::GET,
						&format!("/channels/{parent}/messages/{thread}"),
						None,
					)
					.await
					.and_then(|bytes| {
						decode::<MessageDto>(&bytes)
							.map(MessageDto::into_model)
							.map_err(|_| Failure::Protocol)
					})
					.and_then(|message| {
						if message.id == thread && message.channel == parent {
							Ok(message)
						} else {
							Err(Failure::Protocol)
						}
					});
				Event::ThreadStarter {
					thread,
					request,
					result,
				}
			}
			Command::Pins {
				channel,
				before,
				request,
			} => {
				let result = self.pins(channel, before).await;
				Event::Search {
					channel,
					request,
					result,
				}
			}
			Command::Search {
				channel,
				guild,
				query,
				before,
				offset,
				oldest,
				request,
			} => {
				let result = self
					.search(channel, guild, &query, before, offset, oldest)
					.await;
				Event::Search {
					channel,
					request,
					result,
				}
			}
			Command::CancelSearch | Command::CancelMentions | Command::CancelReplyPreviews => {
				Event::Failure(Failure::Protocol)
			}
			Command::Gifs { query, request } => Event::Gifs {
				request,
				result: self.gifs(query.as_deref()).await,
			},
			Command::CancelGifs => Event::Failure(Failure::Protocol),
			Command::GifFavorites { request, change } => Event::GifFavorites {
				request,
				result: self.gif_favorites(change).await,
			},
			Command::ReactionPreferences { request } => Event::ReactionPreferences {
				request,
				result: self.reaction_preferences().await,
			},
			Command::MarkRead {
				channel,
				message,
				request,
				manual,
				mention_count,
			} => {
				let result = self
					.mark_read(channel, message, manual, mention_count)
					.await;
				Event::ReadState(client_core::read_state::Event::Result {
					channel,
					message,
					request,
					result,
				})
			}
			Command::MarkGuildRead { guild, request } => {
				let result = self.mark_guild_read(guild).await;
				Event::ReadState(client_core::read_state::Event::GuildAck {
					guild,
					request,
					result,
				})
			}
			Command::MarkAllRead { request, channels } => {
				let (acked, result) = self.mark_all_read(&channels).await;
				Event::ReadState(client_core::read_state::Event::AllAck {
					request,
					acked,
					result,
				})
			}
			Command::Polls(command) => {
				use client_core::polls::{Action, Event as E};
				let channel = command.channel;
				let message = command.message;
				let result = async {
					if channel == Id(0) {
						return Err(Failure::Protocol);
					}
					let bytes = match command.action {
						Action::Create(create) => {
							if message.is_some() || !create.valid() {
								return Err(Failure::Protocol);
							}
							let answers: Vec<_> = create
								.answers
								.iter()
								.map(|a| {
									let emoji = a.emoji.as_ref().map(|e| match e.id {
										Some(id) => serde_json::json!({"id": id.to_string()}),
										None => serde_json::json!({"name": e.name}),
									});
									let mut media = serde_json::json!({"text":a.text});
									if let Some(emoji) = emoji {
										media["emoji"] = emoji;
									}
									serde_json::json!({"poll_media":media})
								})
								.collect();
							self.request(
								Method::POST,
								&format!("/channels/{channel}/messages"),
								Some(serde_json::json!({
									"poll":{"question":{"text":create.question},"answers":answers,"duration":create.duration,
									"allow_multiselect":create.multiselect,"layout_type":1}, "nonce":command.nonce,
									"allowed_mentions":{"parse":[],"replied_user":false}
								})),
							)
							.await?
						}
						action => {
							let id = message.filter(|id| *id != Id(0)).ok_or(Failure::Protocol)?;
							match action {
								Action::End => {
									return self
										.request(
											Method::POST,
											&format!("/channels/{channel}/polls/{id}/expire"),
											None,
										)
										.await
										.and_then(|bytes| {
											decode::<MessageDto>(&bytes)
												.map(MessageDto::into_model)
												.map_err(|_| Failure::Protocol)
										});
								}
								Action::Vote(answers) => {
									if answers.len() > model::polls::MAX_ANSWERS
										|| answers
											.iter()
											.enumerate()
											.any(|(i, a)| *a == 0 || answers[..i].contains(a))
									{
										return Err(Failure::Protocol);
									}
									self.request(
										Method::PUT,
										&format!("/channels/{channel}/polls/{id}/answers/@me"),
										Some(serde_json::json!({"answer_ids":answers})),
									)
									.await?;
								}
								Action::Read => {}
								Action::Create(_) => unreachable!(),
							}
							let bytes = self
								.request(
									Method::GET,
									&format!("/channels/{channel}/messages?limit=1&around={id}"),
									None,
								)
								.await?;
							let [dto] =
								decode::<[MessageDto; 1]>(&bytes).map_err(|_| Failure::Protocol)?;
							if dto.id != id || dto.channel_id != channel {
								return Err(Failure::Protocol);
							}
							return Ok(dto.into_model());
						}
					};
					decode::<MessageDto>(&bytes)
						.map(MessageDto::into_model)
						.map_err(|_| Failure::Protocol)
				}
				.await;
				Event::Polls(E::Result {
					channel,
					message,
					request: command.request,
					result: result.map(Box::new),
				})
			}
			Command::Reactions(command) => {
				use client_core::reactions::{Command as R, Event as E};
				Event::Reactions(match command {
					R::Read {
						channel,
						message,
						request,
					} => {
						let result = self
							.request(
								Method::GET,
								&format!("/channels/{channel}/messages?limit=1&around={message}"),
								None,
							)
							.await
							.and_then(|bytes| {
								// Normal-user sessions read a message through history, not
								// the bot-only single-message endpoint. Never use a neighbor.
								let [dto] = decode::<[MessageDto; 1]>(&bytes)
									.map_err(|_| Failure::Protocol)?;
								if dto.id != message || dto.channel_id != channel {
									return Err(Failure::Protocol);
								}
								Ok(dto.into_model().reactions.unwrap_or_default())
							});
						E::Read {
							channel,
							message,
							request,
							result,
						}
					}
					R::Set {
						channel,
						message,
						emoji,
						add,
						request,
					} => {
						let result = match reaction_path(channel, message, &emoji) {
							Some(path) => self
								.request(
									if add { Method::PUT } else { Method::DELETE },
									&path,
									None,
								)
								.await
								.map(|_| ()),
							None => Err(Failure::Protocol),
						};
						E::Written {
							channel,
							message,
							request,
							result,
						}
					}
					R::Users {
						channel,
						message,
						emoji,
						after,
						request,
					} => {
						let result = match reaction_users_path(channel, message, &emoji, after) {
							Some(path) => {
								self.request(Method::GET, &path, None)
									.await
									.and_then(|bytes| {
										let users = decode::<Vec<UserDto>>(&bytes)
											.map_err(|_| Failure::Protocol)?;
										if users.len() > client_core::reactions::REACTION_USER_PAGE
										{
											return Err(Failure::Protocol);
										}
										Ok(users.into_iter().map(UserDto::into_model).collect())
									})
							}
							None => Err(Failure::Protocol),
						};
						E::Users {
							channel,
							message,
							emoji,
							request,
							result,
						}
					}
				})
			}
			Command::Profile {
				user,
				guild,
				request,
				with_mutuals,
				cosmetics,
			} => {
				let mut path = format!(
					"/users/{user}/profile?with_mutual_guilds={with_mutuals}&with_mutual_friends={with_mutuals}&with_mutual_friends_count=false"
				);
				if let Some(guild) = guild {
					path.push_str(&format!("&guild_id={guild}"));
				}
				let result = async {
					let bytes = self
						.request_limited(Method::GET, &path, None, profile::MAX_PROFILE_WIRE)
						.await?;
					let mut profile = profile::decode_profile(&bytes, guild, with_mutuals)
						.map_err(|_| Failure::Protocol)?;
					if profile.user.id != user {
						return Err(Failure::Protocol);
					}
					// Optional names/artwork must not hold the entire profile behind slow reads
					// or a shared service cooldown. Dropping these reads is safe: they never write.
					let effect = profile.effect_sku.filter(|_| cosmetics.effects);
					let frame = profile.frame_sku.filter(|_| cosmetics.frames);
					let (board, catalog, badges) = tokio::join!(
						tokio::time::timeout(
							PROFILE_EXTRAS,
							self.load_profile_board_games(&mut profile),
						),
						tokio::time::timeout(
							PROFILE_EXTRAS,
							self.load_profile_collectibles(effect, frame),
						),
						// Only the account's own profile is requested without mutuals.
						tokio::time::timeout(
							Duration::from_secs(2),
							self.load_badge_catalog(user, !with_mutuals)
						),
					);
					if let Ok(result) = board {
						result?;
					}
					if let Ok(result) = badges {
						profile.badge_catalog = result?.map(Box::new);
						if !profile.valid() {
							profile.badge_catalog = None;
						}
					}
					if let Ok(result) = catalog {
						let (effect, frame) = result?;
						profile.effect = effect.map(Box::new);
						profile.frame = frame.map(Box::new);
						// Artwork that would exceed the profile bound is dropped, not the profile.
						if !profile.valid() {
							profile.effect = None;
							profile.frame = None;
						}
					}
					Ok(Box::new(profile))
				}
				.await;
				Event::Profile {
					user,
					guild,
					request,
					result,
				}
			}
			Command::BadgeSettings { request, settings } => Event::BadgeSettings {
				request,
				result: self.save_badge_settings(settings).await,
			},
			Command::StreamPreview {
				guild,
				channel,
				user,
				request,
			} => {
				let key = format!("guild:{guild}:{channel}:{user}");
				let result = self
					.request_limited(Method::GET, &format!("/streams/{key}/preview"), None, 4096)
					.await
					.and_then(|bytes| stream_preview_url(&bytes, &key));
				Event::StreamPreview {
					guild,
					channel,
					user,
					request,
					result,
				}
			}
			Command::EditProfile {
				user,
				request,
				changes,
			} => Event::ProfileEdited {
				user,
				request,
				result: self.edit_profile(user, changes).await,
			},
			Command::AccountOverview { user, request } => Event::AccountOverview {
				user,
				request,
				result: self.account_overview(user).await,
			},
			Command::OwnedCosmetics {
				user,
				guild,
				request,
				change,
				nitro,
			} => Event::OwnedCosmetics(client_core::owned_cosmetics::Event {
				user,
				guild,
				request,
				result: self.owned_cosmetics(user, guild, change, nitro).await,
			}),
			Command::AccountConnections {
				user,
				request,
				change,
			} => Event::AccountConnections {
				user,
				request,
				result: if user.0 == 0 {
					Err(Failure::Protocol)
				} else {
					self.account_connections(change).await
				},
			},
			Command::ServerProfile {
				user,
				guild,
				request,
				edit,
			} => Event::ServerProfile(client_core::server_profile::Event {
				user,
				guild,
				request,
				result: self.own_server_profile(user, guild, edit).await,
			}),
			Command::CancelProfile => Event::Failure(Failure::Protocol),
			Command::MemberSearch(_)
			| Command::MemberSearchLocal(_)
			| Command::Voice(_)
			| Command::Members { .. } => Event::Failure(Failure::Protocol),
			Command::History {
				channel,
				before,
				after,
				request,
			} => self.history(channel, before, after, request).await,
			Command::HistoryAround {
				channel,
				target,
				request,
			} => self.history_around(channel, target, request).await,
			Command::ReplyPreview {
				channel,
				message,
				request,
			} => self.reply_preview(channel, message, request).await,
			// The same request a click sends, but it gives way: without spare capacity or during
			// a cooldown it fails with Capacity at once, and a click that adopted it sends its own.
			Command::Prefetch(command) => {
				Box::pin(SPECULATIVE.scope((), self.execute(*command))).await
			}
			Command::Soundboard(command) => {
				use client_core::soundboard::{Event as E, MAX_WIRE_BYTES, Request as R};
				Event::Soundboard(match command {
					R::Default { request } => E::Default {
						request,
						result: self
							.request_limited(
								Method::GET,
								"/soundboard-default-sounds",
								None,
								MAX_WIRE_BYTES,
							)
							.await
							.and_then(|bytes| {
								discord_protocol::soundboard::default_sounds(&bytes)
									.map_err(|_| Failure::Protocol)
							}),
					},
					R::Guild { guild, request } => E::Guild {
						request,
						guild,
						result: if guild.0 == 0 {
							Err(Failure::Protocol)
						} else {
							self.request_limited(
								Method::GET,
								&format!("/guilds/{guild}/soundboard-sounds"),
								None,
								MAX_WIRE_BYTES,
							)
							.await
							.and_then(|bytes| {
								discord_protocol::soundboard::guild_sounds(&bytes, guild)
									.map_err(|_| Failure::Protocol)
							})
						},
					},
					R::Send {
						request,
						channel,
						sound,
						source_guild,
						..
					} => E::Sent {
						request,
						channel,
						sound,
						result: if channel.0 == 0
							|| sound.0 == 0 || source_guild.is_some_and(|id| id.0 == 0)
						{
							Err(Failure::Protocol)
						} else {
							let mut body = serde_json::json!({"sound_id": sound});
							if let Some(guild) = source_guild {
								body["source_guild_id"] = serde_json::json!(guild);
							}
							self.request_limited(
								Method::POST,
								&format!("/channels/{channel}/send-soundboard-sound"),
								Some(body),
								4096,
							)
							.await
							.map_err(|failure| {
								if matches!(
									failure,
									Failure::Network | Failure::Protocol | Failure::Capacity
								) {
									Failure::Ambiguous
								} else {
									failure
								}
							})
							.and_then(|body| {
								if body.is_empty() {
									Ok(())
								} else {
									Err(Failure::Ambiguous)
								}
							})
						},
					},
				})
			}
			Command::StickerPacks => Event::StickerPacks(
				self.request_limited(
					Method::GET,
					"/sticker-packs",
					None,
					client_core::stickers::MAX_PACK_BYTES,
				)
				.await
				.and_then(|b| {
					discord_protocol::stickers::sticker_packs(&b).map_err(|_| Failure::Protocol)
				}),
			),
			Command::Sticker(id) => Event::Sticker {
				id,
				result: if id.0 == 0 {
					Err(Failure::Protocol)
				} else {
					self.request_limited(Method::GET, &format!("/stickers/{id}"), None, 16 * 1024)
						.await
						.and_then(|b| {
							discord_protocol::stickers::sticker(&b).map_err(|_| Failure::Protocol)
						})
				},
			},
			Command::Forward {
				source,
				message,
				guild,
				channel,
				nonce,
			} => {
				let result = if source.0 == 0
					|| message.0 == 0
					|| channel.0 == 0
					|| guild.is_some_and(|id| id.0 == 0)
				{
					Err(Failure::Protocol)
				} else {
					let mut reference =
						serde_json::json!({"type":1,"channel_id":source,"message_id":message});
					if let Some(guild) = guild {
						reference["guild_id"] = serde_json::json!(guild);
					}
					self.request(Method::POST, &format!("/channels/{channel}/messages"), Some(serde_json::json!({
						"message_reference":reference,"nonce":nonce,"allowed_mentions":{"parse":[],"replied_user":false}
					}))).await.and_then(|bytes| {
						let message = decode::<MessageDto>(&bytes).map_err(|_| Failure::Ambiguous)?;
						if message.channel_id != channel { return Err(Failure::Ambiguous); }
						Ok(message.into_model())
					})
				};
				Event::SendResult { nonce, result }
			}
			Command::Send {
				sticker,
				channel,
				content,
				nonce,
				reply,
			} => {
				self.send_text(channel, &content, nonce, reply, sticker, None)
					.await
			}
			Command::VerifiedSend {
				sticker,
				channel,
				content,
				nonce,
				reply,
				captcha,
			} => {
				let target = client_core::captcha::Target::Message {
					channel,
					nonce: nonce.clone(),
				};
				if captcha.matches_target(&target) {
					self.send_text(channel, &content, nonce, reply, sticker, Some(&captcha))
						.await
				} else {
					Event::SendResult {
						nonce,
						result: Err(Failure::ProtocolAt(
							"Verification expired; retry the message",
						)),
					}
				}
			}
			Command::Edit {
				request,
				channel,
				message,
				content,
			} => {
				if content.trim().is_empty()
					|| content.chars().count() > client_core::MAX_PREMIUM_CONTENT
				{
					return Event::Edited {
						request,
						channel,
						message,
						result: Err(Failure::Capacity),
					};
				}
				let body = serde_json::json!({"content": content, "allowed_mentions": allowed_mentions(&content, None)});
				let result = self
					.request(
						Method::PATCH,
						&format!("/channels/{channel}/messages/{message}"),
						Some(body),
					)
					.await
					.and_then(|bytes| {
						decode::<MessageDto>(&bytes)
							.map(MessageDto::into_model)
							.map_err(|_| Failure::Ambiguous)
					});
				Event::Edited {
					request,
					channel,
					message,
					result,
				}
			}
			Command::Delete { channel, message } => {
				match self
					.request(
						Method::DELETE,
						&format!("/channels/{channel}/messages/{message}"),
						None,
					)
					.await
				{
					Ok(_) => Event::Delete {
						channel,
						id: message,
					},
					Err(Failure::Forbidden) => Event::Unavailable(channel),
					Err(f) => Event::Failure(f),
				}
			}
			Command::Pin {
				request,
				channel,
				message,
				pinned,
			} => {
				// Documented message pin routes; a failed pin never affects channel access.
				let result = self
					.request(
						if pinned { Method::PUT } else { Method::DELETE },
						&format!("/channels/{channel}/messages/pins/{message}"),
						None,
					)
					.await
					.map(|_| ());
				Event::Pinned {
					request,
					channel,
					message,
					pinned,
					result,
				}
			}
		}
	}
}
