//! Visible, bounded invite directory using the shared administration and creation lanes.
use crate::{avatars::Avatars, design, dialog, icons, server_invite::InviteDialog};
use client_core::{Command, State};
use egui::{RichText, Vec2};
use model::{
	Id, server_admin,
	server_invites::{Action, Invite},
};

#[derive(Default)]
pub(super) struct InvitesUi {
	create: Option<InviteDialog>,
	channel: Option<Id>,
	creation_pending: bool,
	refresh: bool,
	revoke: Option<String>,
	revoking: bool,
	copied: Option<(String, f64)>,
}

impl InvitesUi {
	pub fn busy(&self) -> bool {
		self.creation_pending
	}
	pub fn overlay_open(&self) -> bool {
		self.create.is_some() || self.revoke.is_some()
	}
	pub fn sync(&mut self, state: &State, guild: Id) {
		if self.creation_pending && !state.server_action_pending() {
			self.creation_pending = false;
			self.refresh |= state.created_invite(guild).is_some();
		}
		if self.revoking && !state.server_admin.pending {
			self.revoking = false;
			if state.server_admin.error.is_none() {
				self.revoke = None;
			}
		}
	}
	pub fn load(&mut self, state: &mut State, guild: Id) -> Option<Command> {
		if state.server_admin.pending {
			return None;
		}
		let missing =
			state.server_admin.guild != Some(guild) || state.server_admin.invites.is_none();
		if !self.refresh && (!missing || state.server_admin.error.is_some()) {
			return None;
		}
		let command =
			state.request_server_admin(guild, server_admin::Action::Invites(Action::Load));
		if command.is_some() {
			self.refresh = false;
		}
		command
	}
	pub fn show(
		&mut self,
		ui: &mut egui::Ui,
		state: &mut State,
		guild: Id,
		avatars: &mut Avatars,
		commands: &mut Vec<Command>,
	) {
		let colors = design::palette(ui);
		let mut action = None;
		ui.horizontal(|ui| {
			ui.label(design::semibold(ui, "Invites", 22.0));
			if ui
				.add_enabled(
					!state.server_admin.pending,
					egui::Button::new("Reload").frame(false),
				)
				.on_hover_text("Reload Invites")
				.clicked()
			{
				action = Some(Action::Load);
			}
		});
		ui.add_space(36.0);
		let paused = state
			.server_admin
			.invites
			.as_ref()
			.is_some_and(|snapshot| snapshot.paused());
		let writable = !state.server_admin.pending
			&& !state.server_settings.saving
			&& !state.server_admin.needs_refresh
			&& !state.server_action_pending();
		let width = ui.available_width();
		ui.horizontal_wrapped(|ui| {
			let label_width = if width >= 550.0 { width - 312.0 } else { width };
			ui.allocate_ui_with_layout(
				Vec2::new(label_width, 38.0),
				egui::Layout::left_to_right(egui::Align::Center),
				|ui| {
					ui.set_width(label_width);
					ui.label(design::eyebrow(
						ui,
						if paused {
							"INVITE LINKS PAUSED"
						} else {
							"ACTIVE INVITE LINKS"
						},
						colors.muted,
					));
				},
			);
			if state.can_pause_guild_invites(guild)
				&& ui
					.add_enabled(
						writable,
						egui::Button::new(
							RichText::new(if paused {
								"Resume Invites"
							} else {
								"Pause Invites"
							})
							.color(if paused { colors.text } else { colors.danger }),
						)
						.min_size(Vec2::new(136.0, 38.0)),
					)
					.clicked()
			{
				action = Some(Action::SetPaused { paused: !paused });
			}
			if state.invite_channel(guild).is_some()
				&& ui
					.add_enabled_ui(writable && !paused, |ui| {
						design::button(ui, "Create Invite Link", design::ButtonKind::Primary)
					})
					.inner
					.clicked()
			{
				state.clear_server_action_result(guild);
				self.channel = state.invite_channel(guild);
				let mut dialog = InviteDialog::default();
				dialog.open();
				self.create = Some(dialog);
			}
		});
		ui.add_space(18.0);
		if let Some(error) = state.server_admin.error {
			design::notice(ui, design::Level::Error, error);
			if ui
				.add_enabled(
					!state.server_admin.pending,
					egui::Button::new("Reload Invites"),
				)
				.clicked()
			{
				action = Some(Action::Load);
			}
		}
		if state.server_admin.pending {
			ui.horizontal(|ui| {
				ui.spinner();
				ui.weak(if state.server_admin.saving {
					"Updating invites…"
				} else {
					"Loading invites…"
				});
			});
		}
		if let Some(snapshot) = &state.server_admin.invites {
			if snapshot.items.is_empty() {
				ui.add_space(32.0);
				ui.label(design::semibold(ui, "No active invite links", 18.0));
				if state.invite_channel(guild).is_some() {
					ui.weak("Create an invite link to welcome people to this server.");
				}
			} else {
				let now = time::OffsetDateTime::now_utc().unix_timestamp_nanos();
				let mut ticking = false;
				if width < 640.0 {
					ui.spacing_mut().scroll = egui::style::ScrollStyle::solid();
					ui.spacing_mut().scroll.foreground_color = true;
				}
				egui::ScrollArea::horizontal()
					.id_salt("invites-table-x")
					.scroll_bar_visibility(if width < 640.0 {
						egui::scroll_area::ScrollBarVisibility::AlwaysVisible
					} else {
						egui::scroll_area::ScrollBarVisibility::VisibleWhenNeeded
					})
					.auto_shrink([false, true])
					.show(ui, |ui| {
						let table_width = width.max(640.0);
						ui.set_width(table_width);
						let columns = [0.30, 0.24, 0.11, 0.18, 0.11, 0.06];
						let (header, _) = ui.allocate_exact_size(
							Vec2::new(table_width, 30.0),
							egui::Sense::hover(),
						);
						for (index, label) in ["Inviter", "Invite Code", "Uses", "Expires", "Roles"]
							.into_iter()
							.enumerate()
						{
							cell(ui, column(header, &columns, index), |ui| {
								ui.add(
									egui::Label::new(design::medium(ui, label, 14.0)).truncate(),
								);
							});
						}
						let height = design::list_height(ui, 0.0);
						ui.spacing_mut().item_spacing.y = 0.0;
						egui::ScrollArea::vertical()
							.id_salt("invites-table-y")
							.max_height(height)
							.auto_shrink([false, true])
							.show_rows(ui, 62.0, snapshot.items.len(), |ui, range| {
								for invite in &snapshot.items[range] {
									ui.push_id(&invite.code, |ui| {
										let (rect, _) = ui.allocate_exact_size(
											Vec2::new(table_width, 62.0),
											egui::Sense::hover(),
										);
										let hovered = ui.rect_contains_pointer(rect);
										if hovered {
											ui.painter().rect_filled(
												rect.shrink(1.0),
												6,
												colors.raised,
											);
										}
										ui.painter().hline(
											rect.x_range(),
											rect.bottom(),
											egui::Stroke::new(1.0, colors.border),
										);
										cell(ui, column(rect, &columns, 0), |ui| {
											ui.horizontal(|ui| {
												if let Some(user) = &invite.inviter {
													avatars.show(ui, user, 28.0, state.demo);
												} else {
													let (rect, _) = ui.allocate_exact_size(
														Vec2::splat(28.0),
														egui::Sense::hover(),
													);
													icons::paint(
														ui.painter(),
														icons::Icon::People,
														rect,
														colors.muted,
													);
												}
												let width = (ui.available_width() - 8.0).max(1.0);
												ui.allocate_ui_with_layout(
													Vec2::new(width, 40.0),
													egui::Layout::top_down(egui::Align::Min),
													|ui| {
														ui.set_width(width);
														ui.spacing_mut().item_spacing.y = 1.0;
														let name =
															invite.inviter.as_ref().map_or(
																"Unknown inviter",
																|user| user.name.as_str(),
															);
														ui.add(
															egui::Label::new(design::medium(
																ui, name, 14.0,
															))
															.truncate(),
														)
														.on_hover_text(name);
														let channel = invite
															.channel_name
															.as_deref()
															.or_else(|| {
																invite
																	.channel
																	.and_then(|id| {
																		state.channel(id)
																	})
																	.map(|channel| {
																		channel.name.as_str()
																	})
															});
														if let Some(channel) = channel {
															ui.add(
																egui::Label::new(
																	RichText::new(format!(
																		"#{channel}"
																	))
																	.size(11.0)
																	.color(colors.muted),
																)
																.truncate(),
															)
															.on_hover_text(channel);
														}
													},
												);
											});
										});
										cell(ui, column(rect, &columns, 1), |ui| {
											let copied = self.copied.as_ref().is_some_and(
												|(code, until)| {
													code == &invite.code
														&& ui.input(|input| input.time) < *until
												},
											);
											if ui
												.add(
													egui::Label::new(
														RichText::new(if copied {
															"Copied!"
														} else {
															&invite.code
														})
														.monospace()
														.size(13.0),
													)
													.truncate()
													.sense(egui::Sense::click()),
												)
												.on_hover_text("Copy invite link")
												.clicked()
											{
												ui.ctx().copy_text(format!(
													"https://discord.gg/{}",
													invite.code
												));
												self.copied = Some((
													invite.code.clone(),
													ui.input(|input| input.time) + 2.0,
												));
												ui.ctx().request_repaint_after(
													std::time::Duration::from_secs(2),
												);
											}
										});
										cell(ui, column(rect, &columns, 2), |ui| {
											ui.add(
												egui::Label::new(
													RichText::new(uses(invite))
														.monospace()
														.size(13.0),
												)
												.truncate(),
											);
										});
										let (expiry, active) = expiry(invite, now);
										ticking |= active;
										cell(ui, column(rect, &columns, 3), |ui| {
											ui.add(
												egui::Label::new(
													RichText::new(&expiry).monospace().size(13.0),
												)
												.truncate(),
											)
											.on_hover_text(expiry);
										});
										if let Some(roles) = &invite.roles {
											let names = roles
												.iter()
												.map(|id| {
													state
														.permissions
														.guilds
														.get(&guild)
														.and_then(|guild| guild.roles.as_ref())
														.and_then(|roles| {
															roles.iter().find(|role| role.id == *id)
														})
														.map_or_else(
															|| id.to_string(),
															|role| role.name.clone(),
														)
												})
												.collect::<Vec<_>>()
												.join(", ");
											cell(ui, column(rect, &columns, 4), |ui| {
												ui.add(
													egui::Label::new(
														RichText::new(&names).size(12.0),
													)
													.truncate(),
												)
												.on_hover_text(names);
											});
										}
										if state.can_revoke_guild_invite(guild, &invite.code) {
											cell(ui, column(rect, &columns, 5), |ui| {
												let (rect, response) = ui.allocate_exact_size(
													Vec2::splat(24.0),
													egui::Sense::click(),
												);
												response.widget_info(|| {
													egui::WidgetInfo::labeled(
														egui::Role::Button,
														writable,
														"Revoke invite",
													)
												});
												if hovered
													|| response.has_focus() || response.hovered()
												{
													icons::paint(
														ui.painter(),
														icons::Icon::Close,
														rect.shrink(4.0),
														if writable {
															colors.text
														} else {
															colors.muted
														},
													);
												}
												if response.on_hover_text("Revoke invite").clicked()
													&& writable
												{
													self.revoke = Some(invite.code.clone());
												}
											});
										}
									});
								}
							});
					});
				if ticking {
					ui.ctx()
						.request_repaint_after(std::time::Duration::from_secs(1));
				}
			}
		} else if !state.server_admin.pending
			&& state.server_admin.error.is_none()
			&& ui.button("Load Invites").clicked()
		{
			action = Some(Action::Load);
		}
		if let Some(action) = action
			&& let Some(command) =
				state.request_server_admin(guild, server_admin::Action::Invites(action))
		{
			commands.push(command);
		}
	}
	pub fn overlays(
		&mut self,
		ctx: &egui::Context,
		state: &mut State,
		guild: Id,
		avatars: &mut Avatars,
		commands: &mut Vec<Command>,
	) {
		if let Some(dialog) = &mut self.create {
			let name = state
				.guild(guild)
				.map_or("Server", |guild| guild.name.as_str())
				.to_owned();
			let close = dialog.show(
				ctx,
				state,
				(guild, &name),
				&mut self.channel,
				avatars,
				commands,
			);
			self.creation_pending |= state.server_action_pending();
			if close {
				self.create = None;
				self.channel = None;
			}
		}
		if let Some(code) = &self.revoke {
			if !state.can_revoke_guild_invite(guild, code) {
				self.revoke = None;
				return;
			}
			let mut confirm = dialog::Confirm::new(
				"revoke-server-invite",
				"Revoke invite?",
				format!(
					"People will no longer be able to join this server with discord.gg/{code}."
				),
			)
			.danger()
			.confirm_label("Revoke Invite")
			.enabled(!state.server_admin.pending && !state.server_admin.needs_refresh);
			if let Some(error) = state.server_admin.error {
				confirm = confirm.note(dialog::Level::Error, error);
			}
			let choice = confirm.show(ctx);
			if choice == Some(dialog::Choice::Confirmed)
				&& let Some(command) = state.request_server_admin(
					guild,
					server_admin::Action::Invites(Action::Revoke { code: code.clone() }),
				) {
				commands.push(command);
				self.revoking = true;
			}
			if choice == Some(dialog::Choice::Cancelled) && !state.server_admin.saving {
				self.revoke = None;
			}
		}
	}
}
fn column(rect: egui::Rect, columns: &[f32; 6], index: usize) -> egui::Rect {
	let x = rect.left() + rect.width() * columns[..index].iter().sum::<f32>();
	egui::Rect::from_min_size(
		egui::pos2(x, rect.top()),
		Vec2::new(rect.width() * columns[index], rect.height()),
	)
}
fn cell(ui: &mut egui::Ui, rect: egui::Rect, content: impl FnOnce(&mut egui::Ui)) {
	ui.scope_builder(
		egui::UiBuilder::new()
			.max_rect(rect.shrink2(Vec2::new(4.0, 0.0)))
			.layout(egui::Layout::left_to_right(egui::Align::Center)),
		|ui| {
			ui.set_width((rect.width() - 8.0).max(1.0));
			ui.set_clip_rect(ui.clip_rect().intersect(rect));
			content(ui);
		},
	);
}
fn uses(invite: &Invite) -> String {
	match (invite.uses, invite.max_uses) {
		(Some(used), Some(max)) if max > 0 => format!("{used}/{max}"),
		(Some(used), _) => used.to_string(),
		(None, _) => "Unknown".into(),
	}
}
fn expiry(invite: &Invite, now: i128) -> (String, bool) {
	let expires = invite.expires_at.or_else(|| {
		invite
			.created_at
			.zip(invite.max_age.filter(|age| *age > 0))
			.and_then(|(created, age)| created.checked_add(i128::from(age) * 1_000_000_000))
	});
	if let Some(expires) = expires {
		let seconds = expires
			.saturating_sub(now)
			.max(0)
			.saturating_add(999_999_999)
			/ 1_000_000_000;
		if seconds == 0 {
			return ("Expired".into(), false);
		}
		let clock = format!(
			"{:02}:{:02}:{:02}",
			seconds / 3600 % 24,
			seconds / 60 % 60,
			seconds % 60
		);
		return (
			if seconds >= 86400 {
				format!("{}:{clock}", seconds / 86400)
			} else {
				clock
			},
			true,
		);
	}
	(
		if invite.max_age == Some(0) {
			"∞"
		} else {
			"Unknown"
		}
		.into(),
		false,
	)
}

#[cfg(test)]
mod tests {
	use super::*;
	fn invite() -> Invite {
		Invite {
			code: "synthetic".into(),
			inviter: None,
			channel: None,
			channel_name: None,
			uses: Some(2),
			max_uses: Some(0),
			max_age: Some(0),
			created_at: None,
			expires_at: None,
			temporary: None,
			roles: None,
		}
	}
	#[test]
	fn expiration_distinguishes_unlimited_unknown_expired_and_day_countdowns() {
		let mut row = invite();
		assert_eq!(expiry(&row, 0), ("∞".into(), false));
		row.max_age = None;
		assert_eq!(expiry(&row, 0), ("Unknown".into(), false));
		row.created_at = Some(10_000_000_000);
		row.max_age = Some(90061);
		assert_eq!(expiry(&row, 10_000_000_000), ("1:01:01:01".into(), true));
		row.expires_at = Some(11_000_000_000);
		assert_eq!(expiry(&row, 10_000_000_001), ("00:00:01".into(), true));
		assert_eq!(expiry(&row, 11_000_000_000), ("Expired".into(), false));
		assert_eq!(uses(&row), "2");
		row.max_uses = Some(10);
		assert_eq!(uses(&row), "2/10");
	}
	fn text_position(shape: &egui::Shape, target: &str) -> Option<egui::Pos2> {
		match shape {
			egui::Shape::Text(text) if text.galley.job.text == target => {
				Some(text.pos + text.galley.rect.size() / 2.0)
			}
			egui::Shape::Vec(shapes) => {
				shapes.iter().find_map(|shape| text_position(shape, target))
			}
			_ => None,
		}
	}
	fn frame(
		ctx: &egui::Context,
		view: &mut InvitesUi,
		state: &mut State,
		commands: &mut Vec<Command>,
		events: Vec<egui::Event>,
		target: &str,
	) -> (Option<egui::Pos2>, Option<String>) {
		let guild = state.guilds[0].id;
		let output = ctx.run_ui(
			egui::RawInput {
				screen_rect: Some(egui::Rect::from_min_size(
					egui::Pos2::ZERO,
					Vec2::new(850.0, 900.0),
				)),
				events,
				..Default::default()
			},
			|ui| {
				ui.set_width(800.0);
				view.show(ui, state, guild, &mut Avatars::default(), commands);
				view.overlays(ui.ctx(), state, guild, &mut Avatars::default(), commands);
			},
		);
		let point = output
			.shapes
			.iter()
			.find_map(|shape| text_position(&shape.shape, target));
		let copied = output.platform_output.commands.iter().find_map(|command| {
			if let egui::OutputCommand::CopyText(text) = command {
				Some(text.clone())
			} else {
				None
			}
		});
		output.drop_without_applying_deltas();
		(point, copied)
	}
	fn click(pos: egui::Pos2) -> Vec<egui::Event> {
		vec![
			egui::Event::PointerMoved(pos),
			egui::Event::PointerButton {
				pos,
				button: egui::PointerButton::Primary,
				pressed: true,
				modifiers: Default::default(),
			},
			egui::Event::PointerButton {
				pos,
				button: egui::PointerButton::Primary,
				pressed: false,
				modifiers: Default::default(),
			},
		]
	}
	#[test]
	fn invite_copy_and_confirmed_revoke_use_existing_command_lane() {
		let ctx = egui::Context::default();
		design::apply(&ctx);
		let mut state = test_support::demo_state();
		state.auth = client_core::auth::AuthState::Authenticated;
		state.gateway_connected = true;
		let guild = state.guilds[0].id;
		state.permissions.guilds.insert(
			guild,
			model::permissions::Guild {
				id: guild,
				owner: state.user.as_ref().map(|user| user.id),
				roles: Some(vec![]),
				member: Some(model::permissions::Member {
					roles: vec![],
					timeout_until: None,
				}),
			},
		);
		state.permissions.clear_cache();
		state.server_admin.guild = Some(guild);
		state.server_admin.invites = Some(model::server_invites::Snapshot {
			guild,
			items: vec![invite()],
			features: vec![],
		});
		let mut view = InvitesUi {
			refresh: true,
			..Default::default()
		};
		state.server_settings.saving = true;
		assert!(view.load(&mut state, guild).is_none());
		assert!(
			view.refresh,
			"defer directory refresh until the other save finishes"
		);
		state.server_settings.saving = false;
		let mut commands = vec![];
		let point = frame(
			&ctx,
			&mut view,
			&mut state,
			&mut commands,
			vec![],
			"synthetic",
		)
		.0
		.unwrap();
		let copied = frame(&ctx, &mut view, &mut state, &mut commands, click(point), "").1;
		assert_eq!(copied.as_deref(), Some("https://discord.gg/synthetic"));
		assert!(commands.is_empty());
		assert!(state.can_revoke_guild_invite(guild, "synthetic"));
		view.revoke = Some("synthetic".into());
		// egui first measures a new modal before painting its centered controls.
		frame(&ctx, &mut view, &mut state, &mut commands, vec![], "");
		let point = frame(
			&ctx,
			&mut view,
			&mut state,
			&mut commands,
			vec![],
			"Revoke Invite",
		)
		.0
		.unwrap();
		assert!(commands.is_empty());
		frame(&ctx, &mut view, &mut state, &mut commands, click(point), "");
		let Command::ServerAdmin { action, .. } = commands.pop().unwrap() else {
			panic!("expected administration command");
		};
		assert!(
			matches!(*action, server_admin::Action::Invites(Action::Revoke { code }) if code == "synthetic")
		);
		state.server_admin.pending = false;
		state.permissions.guilds.get_mut(&guild).unwrap().owner = None;
		state.permissions.guilds.get_mut(&guild).unwrap().member = None;
		state.permissions.clear_cache();
		view.overlays(
			&ctx,
			&mut state,
			guild,
			&mut Avatars::default(),
			&mut commands,
		);
		assert!(view.revoke.is_none());
		assert!(commands.is_empty());
	}
}
