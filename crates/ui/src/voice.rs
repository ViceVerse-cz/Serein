use crate::channel_marks::{self, Emphasis};
use crate::design::LazyHover;
use crate::{MessagingUi, design};
use client_core::{
	Command, State,
	auth::AuthState,
	voice::{Participant, Phase, RosterEntry},
};
use egui::RichText;
use model::Id;

/// Local mutes share the 64 per-user volume slots sent to the mixer.
const MAX_USER_MUTES: usize = 64;

impl MessagingUi {
	/// Fixed session overrides; zero IDs are unused slots. A locally muted speaker is mixed at
	/// zero gain, so unmuting restores the volume chosen for them.
	pub fn voice_user_volumes(&self) -> [(u64, u16); 64] {
		let mut values = self
			.voice_user_volumes
			.as_deref()
			.copied()
			.unwrap_or([(0, 100); 64]);
		for user in self.voice_user_muted.iter().copied() {
			if let Some(slot) = values.iter_mut().find(|(id, _)| *id == user) {
				slot.1 = 0;
			} else if let Some(slot) = values.iter_mut().find(|(id, _)| *id == 0) {
				*slot = (user, 0);
			}
		}
		values
	}

	/// Locally muted speakers, for persistence to device settings.
	pub fn voice_user_mutes(&self) -> &[u64] {
		&self.voice_user_muted
	}

	/// Restore persisted local mutes, e.g. at startup.
	pub fn set_voice_user_mutes(&mut self, values: &[u64]) {
		self.voice_user_muted = values
			.iter()
			.copied()
			.filter(|user| *user != 0)
			.take(MAX_USER_MUTES)
			.collect();
	}

	pub(super) fn voice_user_locally_muted(&self, user: Id) -> bool {
		self.voice_user_muted.contains(&user.0)
	}

	fn set_voice_user_locally_muted(&mut self, user: Id, muted: bool) {
		self.voice_user_muted.retain(|id| *id != user.0);
		if muted && self.voice_user_muted.len() < MAX_USER_MUTES {
			self.voice_user_muted.push(user.0);
		}
	}

	/// Non-default volume overrides, for persistence to device settings.
	pub fn voice_user_volume_overrides(&self) -> Vec<(u64, u16)> {
		self.voice_user_volumes
			.as_deref()
			.into_iter()
			.flatten()
			.filter(|(id, volume)| *id != 0 && *volume != 100)
			.copied()
			.collect()
	}

	/// Restore persisted per-user volume overrides, e.g. at startup.
	pub fn set_voice_user_volume_overrides(&mut self, values: &[(u64, u16)]) {
		if values.is_empty() {
			self.voice_user_volumes = None;
			return;
		}
		let mut array = [(0u64, 100u16); 64];
		for (slot, value) in array.iter_mut().zip(values.iter().take(64)) {
			*slot = *value;
		}
		self.voice_user_volumes = Some(Box::new(array));
	}

	fn voice_participant_menu(
		&mut self,
		response: &egui::Response,
		state: &State,
		entry: &RosterEntry,
	) {
		crate::user_menu::popup(response, egui::Popup::default_response_id(response))
			.close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
			.show(|ui| {
				ui.set_width(220.0);
				let id = entry.participant.user.0;
				if state.user.as_ref().is_some_and(|own| own.id.0 != id) {
					let muted = self.voice_user_locally_muted(entry.participant.user);
					if ui
						.button(if muted { "Unmute" } else { "Mute" })
						.on_hover_text(
							"Silence this person on this device only. Nobody else is affected.",
						)
						.clicked()
					{
						self.set_voice_user_locally_muted(entry.participant.user, !muted);
					}
					let mut volume = self
						.voice_user_volumes
						.as_deref()
						.and_then(|values| values.iter().find(|(user, _)| *user == id))
						.map_or(100, |(_, volume)| *volume);
					let changed = gain_slider(ui, &mut volume, "User volume").changed();
					let reset = ui
						.add_enabled(volume != 100, egui::Button::new("Reset volume"))
						.clicked();
					if changed || reset {
						let values = self
							.voice_user_volumes
							.get_or_insert_with(|| Box::new([(0, 100); 64]));
						let index = values
							.iter()
							.position(|(user, _)| *user == id)
							.or_else(|| values.iter().position(|(user, _)| *user == 0))
							.unwrap_or_else(|| {
								values.rotate_left(1);
								63
							});
						values[index] = if reset || volume == 100 {
							(0, 100)
						} else {
							(id, volume)
						};
					}
					ui.separator();
				}
				if let Some(user) = resolve_member(state, entry).0 {
					crate::user_menu::contents(
						ui,
						state,
						user,
						&mut self.profile,
						&mut self.user_action,
						None,
					);
				}
			});
	}

	fn is_speaking(&self, state: &State, channel: Id, participant: &Participant) -> bool {
		!self.voice_user_locally_muted(participant.user)
			&& !participant.muted
			&& !participant.deafened
			&& !participant.server_muted
			&& !participant.server_deafened
			&& state.voice.active.as_ref().is_some_and(|call| {
				call.channel == channel
					&& call.phase == Phase::Connected
					&& !call.deafened
					&& !call.server_deafened
			}) && self.voice_speaking.contains(&participant.user)
	}

	pub(super) fn voice_channel_button(
		&mut self,
		ui: &mut egui::Ui,
		state: &State,
		channel: &model::Channel,
		selected: bool,
		draggable: bool,
	) -> egui::Response {
		let colors = design::palette(ui);
		let call = state
			.voice
			.active
			.as_ref()
			.filter(|c| c.channel == channel.id);
		let connected = call.is_some_and(|c| matches!(c.phase, Phase::Waiting | Phase::Connected));
		let elapsed = call.and_then(elapsed_label);
		let access = state.channel_access(channel.id);
		let viewable = !access.hidden();
		let (rect, response) = ui
			.push_id(channel.id, |ui| {
				ui.allocate_exact_size(
					egui::vec2(ui.available_width(), 34.0),
					if viewable {
						if draggable {
							egui::Sense::click_and_drag()
						} else {
							egui::Sense::click()
						}
					} else {
						egui::Sense::hover()
					},
				)
			})
			.inner;
		let row = rect.shrink2(egui::vec2(0.0, 1.0));
		let hovered = viewable && (response.hovered() || response.has_focus());
		if selected {
			ui.painter().rect_filled(row, 8, colors.selected);
		} else if hovered {
			ui.painter()
				.rect_filled(row, 8, crate::design::row_highlight(ui, colors.hover, 1.0));
		}
		let text_color = channel_marks::tint(
			&colors,
			access,
			if !viewable {
				Emphasis::Unavailable
			} else if connected {
				Emphasis::Connected
			} else if selected || hovered {
				Emphasis::Focused
			} else {
				Emphasis::Idle
			},
		);
		let glyph = egui::Rect::from_center_size(
			row.left_center() + egui::vec2(18.0, 0.0),
			egui::Vec2::splat(20.0),
		);
		crate::icons::paint(ui.painter(), crate::icons::Icon::Speaker, glyph, text_color);
		channel_marks::paint(
			ui.painter(),
			access,
			row,
			glyph,
			text_color,
			if selected {
				colors.selected
			} else if hovered {
				crate::design::row_highlight(ui, colors.hover, 1.0)
			} else {
				colors.sidebar
			},
		);
		let marks = channel_marks::trailing(access);
		let elapsed_width = if elapsed.is_some() { 64.0 } else { 0.0 };
		let name = ui.painter().layout(
			channel.name.clone(),
			egui::FontId::new(15.0, design::medium_family(ui.ctx())),
			text_color,
			(row.width() - 40.0 - elapsed_width - marks).max(10.0),
		);
		let name_rect = egui::Rect::from_min_size(
			egui::pos2(row.left() + 34.0, row.center().y - name.size().y * 0.5),
			egui::vec2(row.width() - 40.0 - elapsed_width - marks, name.size().y),
		);
		ui.painter()
			.with_clip_rect(name_rect)
			.galley(name_rect.min, name, text_color);
		if let Some(elapsed) = &elapsed {
			ui.painter().text(
				row.right_center() - egui::vec2(8.0 + marks, 0.0),
				egui::Align2::RIGHT_CENTER,
				elapsed,
				egui::FontId::monospace(11.0),
				text_color,
			);
		}
		if elapsed.is_some() && ui.is_rect_visible(response.rect) {
			ui.ctx()
				.request_repaint_after(std::time::Duration::from_secs(1));
		}
		response.widget_info(|| {
			egui::WidgetInfo::selected(
				egui::WidgetType::SelectableLabel,
				viewable,
				selected,
				format!(
					"{} voice channel{}{}",
					channel.name,
					channel_marks::label(access),
					if connected { ", connected" } else { "" }
				),
			)
		});
		response.on_hover_text_with(|| {
			format!(
				"{} · View voice channel{}{}",
				channel.name,
				channel_marks::label(access),
				if connected { " · Connected" } else { "" }
			)
		})
	}

	fn remember_profile_trigger(&mut self, rect: egui::Rect, pointer_inside: bool) {
		if pointer_inside {
			self.profile_trigger = Some(rect);
		}
	}

	fn toggle_profile(&mut self, user: &model::User) {
		if self.profile.as_ref().is_some_and(|open| open.id == user.id) {
			self.profile = None;
			self.profile_link = None;
			self.profile_anchor = None;
		} else {
			self.profile = Some(user.clone());
		}
	}

	pub(super) fn voice_participant(
		&mut self,
		ui: &mut egui::Ui,
		state: &State,
		entry: &RosterEntry,
	) {
		if !state.can_view(entry.channel) {
			return;
		}
		let colors = design::palette(ui);
		let (user, name) = resolve_member(state, entry);
		ui.push_id(
			("voice-participant", entry.channel, entry.participant.user),
			|ui| {
				let (rect, row) = ui.allocate_exact_size(
					egui::vec2(ui.available_width(), 34.0),
					egui::Sense::click(),
				);
				row.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, name));
				let hovered = row.contains_pointer() || row.has_focus();
				if hovered {
					ui.painter().rect_filled(
						rect.shrink2(egui::vec2(0.0, 1.0)),
						8,
						crate::design::row_highlight(ui, colors.hover, 1.0),
					);
				}
				let name_color = if hovered {
					colors.text_strong
				} else {
					colors.muted
				};
				let mut inner = ui.new_child(
					egui::UiBuilder::new()
						.max_rect(rect)
						.layout(egui::Layout::left_to_right(egui::Align::Center)),
				);
				inner.spacing_mut().item_spacing.x = 6.0;
				let avatar = match user {
					Some(user) => self.avatars.show_plain(&mut inner, user, 28.0, state.demo),
					None => {
						let (r, response) = inner
							.allocate_exact_size(egui::Vec2::splat(28.0), egui::Sense::hover());
						design::paint_avatar(&inner, name, 28.0, r);
						response.on_hover_text(name)
					}
				};
				if self.is_speaking(state, entry.channel, &entry.participant) {
					speaking_avatar(&inner, &avatar, name);
				}
				let locally_muted = self.voice_user_locally_muted(entry.participant.user);
				inner.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
					if entry.participant.deafened {
						status_icon(
							ui,
							crate::icons::Icon::HeadphonesSlash,
							colors.muted,
							if entry.participant.server_deafened {
								"Deafened by server"
							} else {
								"Deafened"
							},
						);
					}
					if entry.participant.muted {
						status_icon(
							ui,
							crate::icons::Icon::MicrophoneSlash,
							colors.muted,
							if entry.participant.server_muted {
								"Muted by server"
							} else {
								"Microphone muted"
							},
						);
					}
					if locally_muted {
						status_icon(
							ui,
							crate::icons::Icon::Speaker,
							colors.danger,
							"Muted for you on this device",
						);
					}
					if entry.participant.streaming {
						live_badge(ui);
					}
					ui.allocate_ui_with_layout(
						egui::vec2(ui.available_width(), 28.0),
						egui::Layout::left_to_right(egui::Align::Center),
						|ui| {
							ui.add(
								egui::Label::new(RichText::new(name).color(name_color))
									.truncate()
									.selectable(false),
							)
							.on_hover_text(name);
						},
					);
				});
				self.voice_participant_menu(&row, state, entry);
				if let Some(user) = user {
					self.remember_profile_trigger(row.rect, row.contains_pointer());
					if row.clicked() {
						self.toggle_profile(user);
					}
				}
			},
		);
	}

	/// Guild voice channel: Discord-style black stage with participant tiles and, when
	/// connected, the call control bar; otherwise a Join Voice button.
	pub(super) fn voice_channel(
		&mut self,
		ui: &mut egui::Ui,
		state: &mut State,
		channel: Id,
		commands: &mut Vec<Command>,
	) {
		let connected = state
			.voice
			.active
			.as_ref()
			.is_some_and(|call| call.channel == channel);
		let stage = ui.available_rect_before_wrap();
		ui.painter().rect_filled(stage, 0, STAGE_FILL);
		let (rect, _) = ui.allocate_exact_size(stage.size(), egui::Sense::hover());
		let notices = self.stage_notices(state, channel, connected);
		let bottom = CONTROL_HEIGHT + 2.0 * STAGE_MARGIN;
		let body = egui::Rect::from_min_max(
			rect.left_top() + egui::vec2(STAGE_MARGIN, STAGE_MARGIN),
			egui::pos2(rect.right() - STAGE_MARGIN, rect.bottom() - bottom),
		);
		let mut body_ui = ui.new_child(
			egui::UiBuilder::new()
				.max_rect(body)
				.layout(egui::Layout::top_down(egui::Align::Min)),
		);
		stage_notices(&mut body_ui, &notices);
		call_failure(
			&mut body_ui,
			state
				.voice
				.active
				.as_ref()
				.filter(|c| c.channel == channel)
				.and_then(|c| c.error),
			STAGE_TEXT,
		);
		if !state.can_view(channel) {
			body_ui.label(
				RichText::new("Participant list unavailable with the current access.")
					.color(STAGE_MUTED),
			);
		} else {
			let entries = stage_participants(state, channel);
			if entries.is_empty() {
				body_ui.add_space((body_ui.available_height() * 0.4).max(0.0));
				body_ui.vertical_centered(|ui| {
					ui.label(
						design::semibold(
							ui,
							if !state.demo && !state.gateway_connected {
								"Participant list unavailable while disconnected"
							} else {
								"No one's here yet"
							},
							18.0,
						)
						.color(STAGE_TEXT),
					);
				});
			} else {
				if !state.demo && !state.gateway_connected {
					body_ui.label(
						RichText::new("Last known participants · reconnect to refresh")
							.small()
							.color(STAGE_MUTED),
					);
				}
				self.participant_tiles(&mut body_ui, state, channel, &entries, false);
			}
		}
		self.apply_watch_request(state);
		let bar = egui::Rect::from_min_max(
			egui::pos2(rect.left(), rect.bottom() - bottom),
			rect.right_bottom(),
		);
		let mut bar_ui = ui.new_child(egui::UiBuilder::new().max_rect(bar).layout(
			egui::Layout::centered_and_justified(egui::Direction::LeftToRight),
		));
		if connected {
			self.call_controls(&mut bar_ui, state, channel, commands);
		} else {
			bar_ui.horizontal_centered(|ui| {
				ui.add_space((ui.available_width() - 160.0).max(0.0) * 0.5);
				self.call_button(ui, state, channel, commands);
			});
		}
	}

	/// Which stage tiles a channel would show, in stage order.
	fn stage_tiles<'a>(
		&self,
		state: &State,
		channel: Id,
		entries: &'a [RosterEntry],
	) -> Vec<Tile<'a>> {
		let call = state
			.voice
			.active
			.as_ref()
			.filter(|call| call.channel == channel && call.phase != Phase::Failed);
		let mut tiles = Vec::with_capacity(entries.len() + 2);
		if call.is_some_and(|call| {
			self.screen.context == Some((state.generation, channel, call.request))
				&& self.screen.busy
				&& self.screen.preview.is_some()
		}) {
			tiles.push(Tile::LocalScreen);
		}
		if let Some(streamer) = call.and_then(|call| call.watching) {
			tiles.push(Tile::Stream(streamer));
		}
		tiles.extend(entries.iter().map(Tile::Participant));
		tiles
	}

	/// True once any stage tile carries video, so the direct-message stage can grow.
	pub(super) fn stage_shows_video(&self, state: &State, channel: Id) -> bool {
		let entries = stage_participants(state, channel);
		self.stage_tiles(state, channel, &entries)
			.iter()
			.any(|tile| self.tile_has_video(state, channel, tile))
	}

	/// Stage tiles: a best-fit grid, or one enlarged video with the rest in a strip below.
	///
	/// `dm` drops the tile plates while no one shares video, matching Discord's
	/// direct-message calls where idle participants are avatars on the stage.
	fn participant_tiles(
		&mut self,
		ui: &mut egui::Ui,
		state: &State,
		channel: Id,
		entries: &[RosterEntry],
		dm: bool,
	) {
		let tiles = self.stage_tiles(state, channel, entries);
		if tiles.is_empty() {
			return;
		}
		// Focus survives only while that tile still shows video; Escape restores the grid.
		let focus = self.voice_focus.filter(|focus| {
			tiles
				.iter()
				.any(|tile| tile.focus() == *focus && self.tile_has_video(state, channel, tile))
		});
		let focus = focus.filter(|_| !ui.input(|input| input.key_pressed(egui::Key::Escape)));
		self.voice_focus = focus;
		let video = tiles
			.iter()
			.any(|tile| self.tile_has_video(state, channel, tile));
		let frameless = dm && !video && !state.is_group_dm(channel);
		let area = ui.available_rect_before_wrap();
		if area.width() < 40.0 || area.height() < 40.0 {
			return;
		}
		let mut toggle = None;
		if let Some(focus) = focus {
			let index = tiles
				.iter()
				.position(|tile| tile.focus() == focus)
				.expect("validated focus");
			// The enlarged tile keeps the stage; everyone else becomes a small strip below it,
			// only when the pill-bar toggle asks for them.
			let strip = if tiles.len() > 1 && self.voice_focus_participants {
				(area.height() * 0.18).clamp(64.0, 124.0)
			} else {
				0.0
			};
			let main = egui::Rect::from_min_size(
				area.min,
				egui::vec2(
					area.width(),
					(area.height() - strip - if strip > 0.0 { TILE_GAP } else { 0.0 }).max(80.0),
				),
			);
			toggle = self.tile(ui, state, channel, &tiles[index], main, false, true);
			if strip > 0.0 {
				let size = egui::vec2(strip * 16.0 / 9.0, strip);
				let others = (tiles.len() - 1) as f32;
				let row = size.x * others + TILE_GAP * (others - 1.0);
				let mut x = area.center().x - row * 0.5;
				let top = main.bottom() + TILE_GAP;
				for (position, tile) in tiles.iter().enumerate() {
					if position == index {
						continue;
					}
					let rect = egui::Rect::from_min_size(egui::pos2(x, top), size);
					x += size.x + TILE_GAP;
					if !area.intersects(rect) {
						continue;
					}
					if let Some(focus) = self.tile(ui, state, channel, tile, rect, false, false) {
						toggle = Some(focus);
					}
				}
			}
		} else {
			// Pick the column count that makes the tiles largest inside the stage, so two
			// participants fill the width instead of sitting in a corner.
			let cap = if frameless { 148.0 } else { 620.0 };
			let (columns, mut size) = best_fit(tiles.len(), area.size(), cap);
			if frameless {
				size.y = size.y.max(132.0).min(area.height());
			}
			if size.x < 24.0 || size.y < 24.0 {
				return;
			}
			let rows = tiles.len().div_ceil(columns);
			let content = size.y * rows as f32 + TILE_GAP * (rows as f32 - 1.0);
			let top = area.top() + ((area.height() - content) * 0.5).max(0.0);
			for row in 0..rows {
				let first = row * columns;
				let in_row = (tiles.len() - first).min(columns);
				let width = size.x * in_row as f32 + TILE_GAP * (in_row as f32 - 1.0);
				let mut x = area.center().x - width * 0.5;
				let y = top + (size.y + TILE_GAP) * row as f32;
				for tile in &tiles[first..first + in_row] {
					let rect = egui::Rect::from_min_size(egui::pos2(x, y), size);
					x += size.x + TILE_GAP;
					if let Some(focus) = self.tile(ui, state, channel, tile, rect, frameless, false)
					{
						toggle = Some(focus);
					}
				}
			}
		}
		if let Some(focus) = toggle {
			self.voice_focus = (self.voice_focus != Some(focus)).then_some(focus);
		}
	}

	fn remote_texture(&self, user: Id) -> Option<&egui::TextureHandle> {
		self.voice_remote_video
			.iter()
			.find(|(id, _)| *id == user)
			.map(|(_, texture)| texture)
	}

	fn tile_has_video(&self, state: &State, channel: Id, tile: &Tile<'_>) -> bool {
		match tile {
			Tile::LocalScreen => self.screen.preview.is_some(),
			Tile::Stream(_) => true,
			Tile::Participant(entry) => {
				let own = state
					.user
					.as_ref()
					.is_some_and(|user| user.id == entry.participant.user);
				if own {
					self.voice_camera_preview.is_some()
						&& state.voice.active.as_ref().is_some_and(|call| {
							call.channel == channel && call.camera && call.phase != Phase::Failed
						})
				} else {
					entry.participant.video && self.remote_texture(entry.participant.user).is_some()
				}
			}
		}
	}

	/// One stage tile. Returns its focus key when a video tile is clicked to enlarge or restore.
	#[allow(clippy::too_many_arguments)] // Placement and framing flags of one tile.
	fn tile(
		&mut self,
		ui: &mut egui::Ui,
		state: &State,
		channel: Id,
		tile: &Tile<'_>,
		rect: egui::Rect,
		frameless: bool,
		focused: bool,
	) -> Option<StageFocus> {
		let has_video = self.tile_has_video(state, channel, tile);
		let response = ui.interact(
			rect,
			ui.scope_id().with(("voice-tile", tile.key())),
			if has_video || matches!(tile, Tile::Participant(_)) {
				egui::Sense::click()
			} else {
				egui::Sense::hover()
			},
		);
		if !ui.is_rect_visible(rect) {
			return None;
		}
		let compact = rect.height() < 132.0;
		let hint = match tile {
			Tile::LocalScreen => {
				self.screen_tile(ui, rect, compact);
				self.screen
					.capture_status
					.unwrap_or("Your screen · local preview")
			}
			Tile::Stream(streamer) => {
				self.stream_tile(ui, state, rect, channel, *streamer, compact);
				"Screen share you are watching"
			}
			Tile::Participant(entry) => {
				self.participant_tile(ui, state, entry, rect, frameless, compact);
				self.voice_participant_menu(&response, state, entry);
				""
			}
		};
		if has_video {
			let label = if focused {
				"Click or press Escape to return to the grid"
			} else {
				"Click to enlarge"
			};
			response
				.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label));
			let hover = if hint.is_empty() {
				label.to_owned()
			} else {
				format!("{hint} · {label}")
			};
			if response.clicked() {
				return Some(tile.focus());
			}
			response.on_hover_text(hover);
		} else if !hint.is_empty() {
			response.on_hover_text(hint);
		}
		None
	}

	fn screen_tile(&self, ui: &mut egui::Ui, rect: egui::Rect, compact: bool) {
		let content = self
			.screen
			.preview
			.as_ref()
			.map(|texture| {
				let content = fit_rect(rect, texture.size_vec2());
				ui.put(
					content,
					egui::Image::from_texture((texture.id(), content.size())).corner_radius(8),
				);
				content
			})
			.unwrap_or(rect);
		if !compact {
			name_badge(ui, content, "Your screen", None);
		}
	}

	/// The screen share this device chose to watch: the latest decoded picture or a status.
	fn stream_tile(
		&mut self,
		ui: &mut egui::Ui,
		state: &State,
		rect: egui::Rect,
		channel: Id,
		streamer: Id,
		compact: bool,
	) {
		let name = participant_user(state, channel, streamer)
			.map_or_else(|| "Participant".to_owned(), |user| user.name.clone());
		let content = match &self.voice_stream_view {
			Some(texture) => {
				let content = fit_rect(rect, texture.size_vec2());
				ui.put(
					content,
					egui::Image::from_texture((texture.id(), content.size())).corner_radius(8),
				);
				content
			}
			None => {
				ui.painter().rect_filled(rect, 8, TILE_FILL);
				if !compact {
					let status = if self.voice_stream_status.is_empty() {
						"Connecting to the stream…"
					} else {
						self.voice_stream_status
					};
					let spinner = egui::Rect::from_center_size(
						rect.center() - egui::vec2(0.0, 18.0),
						egui::Vec2::splat(24.0),
					);
					ui.put(spinner, egui::Spinner::new().color(STAGE_MUTED));
					ui.painter().text(
						rect.center() + egui::vec2(0.0, 18.0),
						egui::Align2::CENTER_CENTER,
						status,
						egui::FontId::proportional(13.0),
						STAGE_MUTED,
					);
				}
				rect
			}
		};
		if compact {
			return;
		}
		name_badge(ui, content, &format!("{name}'s screen"), None);
		if tile_button(
			ui,
			content,
			"Stop watching",
			egui::Color32::from_black_alpha(170),
			design::palette(ui).danger,
			"Stop receiving this screen share",
		)
		.clicked()
		{
			self.watch_request = Some(None);
		}
	}

	/// Apply a tile's watch click once the stage has mutable state again.
	fn apply_watch_request(&mut self, state: &mut State) {
		match self.watch_request.take() {
			Some(Some(user)) => {
				let _ = state.watch_stream(user);
			}
			Some(None) => state.stop_watching(),
			None => {}
		}
	}

	fn participant_tile(
		&mut self,
		ui: &mut egui::Ui,
		state: &State,
		entry: &RosterEntry,
		rect: egui::Rect,
		frameless: bool,
		compact: bool,
	) {
		let size = rect.size();
		let (user, name) = resolve_member(state, entry);
		let colors = design::palette(ui);
		let own = state
			.user
			.as_ref()
			.is_some_and(|user| user.id == entry.participant.user);
		let call = state
			.voice
			.active
			.as_ref()
			.filter(|call| call.channel == entry.channel && call.phase != Phase::Failed);
		// The local preview is mirrored like a webcam; remote cameras fill the tile edge to edge.
		let video = if own && call.is_some_and(|call| call.camera) {
			self.voice_camera_preview
				.as_ref()
				.map(|texture| (texture.id(), texture.size_vec2(), true))
		} else if entry.participant.video {
			self.remote_texture(entry.participant.user)
				.map(|texture| (texture.id(), texture.size_vec2(), false))
		} else {
			None
		};
		if !frameless {
			ui.painter().rect_filled(rect, 8, TILE_FILL);
		}
		if let Some((id, image, mirror)) = video {
			cover_image(ui, rect, id, image, mirror);
		}
		let speaking = self.is_speaking(state, entry.channel, &entry.participant);
		let avatar_size = if compact {
			(size.y * 0.5).clamp(28.0, 48.0)
		} else if frameless {
			(size.y * 0.58).clamp(56.0, 112.0)
		} else {
			(size.y * 0.42).clamp(48.0, 128.0)
		};
		let offset = if compact {
			0.0
		} else if frameless {
			14.0
		} else {
			10.0
		};
		let avatar_rect = egui::Rect::from_center_size(
			rect.center() - egui::vec2(0.0, offset),
			egui::Vec2::splat(avatar_size),
		);
		let mut avatar_ui = ui.new_child(egui::UiBuilder::new().max_rect(avatar_rect));
		if video.is_some() {
			avatar_ui.set_opacity(0.0);
		}
		let avatar = if let Some(user) = user {
			self.avatars
				.show(&mut avatar_ui, user, avatar_size, state.demo)
		} else {
			design::avatar(&mut avatar_ui, name, avatar_size)
		};
		// Mute state reads as Discord's red ring plus the matching slashed glyph.
		let silenced = if entry.participant.deafened || entry.participant.server_deafened {
			Some(crate::icons::Icon::HeadphonesSlash)
		} else if entry.participant.muted || entry.participant.server_muted {
			Some(crate::icons::Icon::MicrophoneSlash)
		} else if self.voice_user_locally_muted(entry.participant.user) {
			// Silenced on this device only; the speaker glyph separates it from a microphone mute.
			Some(crate::icons::Icon::Speaker)
		} else {
			None
		};
		if let Some(icon) = silenced
			&& video.is_none()
		{
			ui.painter().circle_stroke(
				avatar.rect.center(),
				avatar.rect.width() * 0.5 + 2.0,
				egui::Stroke::new(2.0, colors.danger),
			);
			let badge = avatar.rect.right_bottom() - egui::Vec2::splat(avatar_size * 0.14);
			ui.painter().circle_filled(badge, 12.0, STAGE_FILL);
			ui.painter().circle_filled(badge, 10.0, colors.danger);
			crate::icons::paint(
				ui.painter(),
				icon,
				egui::Rect::from_center_size(badge, egui::Vec2::splat(12.0)),
				egui::Color32::WHITE,
			);
		}
		if silenced.is_none() && speaking {
			if video.is_none() && frameless {
				speaking_avatar(ui, &avatar, name);
			}
			if !frameless {
				ui.painter().rect_stroke(
					rect.shrink(1.0),
					8,
					egui::Stroke::new(2.0, colors.positive),
					egui::StrokeKind::Inside,
				);
			}
		}
		self.voice_participant_menu(&avatar, state, entry);
		if let Some(user) = user {
			self.remember_profile_trigger(avatar.rect, avatar.contains_pointer());
			if avatar.clicked() {
				self.toggle_profile(user);
			}
		}
		// Discord's LIVE pill marks a streamer on every tile size; strip tiles get a small one
		// so it never covers the avatar.
		if entry.participant.streaming {
			let (pill, font) = if compact {
				(egui::vec2(30.0, 15.0), 9.0)
			} else {
				(egui::vec2(40.0, 20.0), 11.0)
			};
			let margin = if compact { 5.0 } else { 8.0 };
			let live = egui::Rect::from_min_size(rect.left_top() + egui::Vec2::splat(margin), pill);
			ui.painter().rect_filled(live, 4, colors.danger);
			ui.painter().text(
				live.center(),
				egui::Align2::CENTER_CENTER,
				"LIVE",
				egui::FontId::new(font, design::medium_family(ui.ctx())),
				egui::Color32::WHITE,
			);
		}
		if compact {
			return;
		}
		// Name label with the mute glyph: a bottom-left badge on plates, centred under the
		// avatar once the plates are gone.
		if frameless {
			let font = egui::FontId::new(13.0, design::medium_family(ui.ctx()));
			let galley =
				ui.painter()
					.layout(name.to_owned(), font, STAGE_TEXT, (size.x - 24.0).max(20.0));
			ui.painter().galley(
				egui::pos2(
					rect.center().x - galley.size().x * 0.5,
					avatar_rect.bottom() + 20.0 - galley.size().y * 0.5,
				),
				galley,
				STAGE_TEXT,
			);
		} else {
			name_badge(ui, rect, name, silenced.filter(|_| video.is_some()));
		}
		// Watching is an explicit click, never automatic.
		if entry.participant.streaming
			&& !own && let Some(call) = call
			&& matches!(call.phase, Phase::Connected | Phase::Waiting)
		{
			let watching = call.watching == Some(entry.participant.user);
			let (label, fill, hint) = if watching {
				(
					"Watching",
					egui::Color32::from_black_alpha(170),
					"Stop receiving this screen share",
				)
			} else {
				(
					"Watch stream",
					colors.accent,
					"Receive this participant's screen share",
				)
			};
			let hover = if watching {
				colors.danger
			} else {
				colors.accent.gamma_multiply(1.2)
			};
			if tile_button(ui, rect, label, fill, hover, hint).clicked() {
				self.watch_request = Some((!watching).then_some(entry.participant.user));
			}
		}
	}

	fn stage_notices(&self, state: &State, channel: Id, connected: bool) -> Vec<(String, bool)> {
		let mut notices = Vec::new();
		if let Some(call) = state.voice.active.as_ref().filter(|c| c.channel == channel) {
			if self.screen.context == Some((state.generation, channel, call.request)) {
				let status = self.screen.capture_status.unwrap_or(self.screen.status);
				if !status.is_empty() {
					notices.push((status.into(), false));
				}
			}
			if !self.voice_camera_status.is_empty() {
				notices.push((self.voice_camera_status.into(), false));
			}
			if call.watching.is_none() && !self.voice_stream_status.is_empty() {
				notices.push((self.voice_stream_status.into(), false));
			}
			if call.server_deafened {
				notices.push(("Deafened by the server".into(), false));
			} else if call.server_muted {
				notices.push(("Muted by the server".into(), false));
			}
			if !state.demo && !state.can_speak(channel) {
				notices.push((
					"Speaking is unavailable in this channel. You can still listen.".into(),
					false,
				));
			} else if !state.demo
				&& state.permission(channel, model::permissions::USE_VAD) != Some(true)
			{
				notices.push((
					"Push-to-talk is required to speak here. Enable it in Voice settings.".into(),
					false,
				));
			}
		} else if !connected {
			if state.demo {
				notices.push((
					"Synthetic participants · microphone and speakers are off.".into(),
					false,
				));
			} else if let Some(reason) = self.call_unavailable(state, channel) {
				notices.push((reason.to_owned(), false));
			}
		}
		notices
	}

	fn call_unavailable(&self, state: &State, channel: Id) -> Option<&'static str> {
		if state.demo {
			Some("Calls are unavailable in the offline preview. No microphone is accessed.")
		} else if !self.voice_available {
			Some("Voice is unavailable in this session.")
		} else if state.auth != AuthState::Authenticated || !state.gateway_connected {
			Some("Reconnect to Discord before calling.")
		} else if state.voice.active.is_some() {
			Some("Leave your current call before starting another.")
		} else if !state.can_call(channel) {
			Some("Joining this channel is unavailable with current permission information.")
		} else {
			None
		}
	}

	pub(super) fn call_button(
		&mut self,
		ui: &mut egui::Ui,
		state: &mut State,
		channel: Id,
		commands: &mut Vec<Command>,
	) -> egui::Response {
		let unavailable = self.call_unavailable(state, channel);
		let incoming = state.voice.incoming == Some(channel);
		let guild = state
			.channels
			.iter()
			.any(|c| c.id == channel && c.kind == 2);
		let label = if guild {
			"Join Voice"
		} else if incoming {
			"Answer call"
		} else if state.voice.has_dm_call(channel) {
			"Join call"
		} else {
			"Start voice call"
		};
		let hint = unavailable.unwrap_or(if state.can_speak(channel) {
			"Join audio. Your microphone starts after the call is secured."
		} else {
			"Join to listen. Speaking is unavailable in this channel."
		});
		// Guild channels keep Discord's green Join Voice button; DM headers use an icon.
		let response = if guild {
			let colors = design::palette(ui);
			ui.add_enabled(
				unavailable.is_none(),
				egui::Button::new(design::medium(ui, label, 15.0).color(egui::Color32::WHITE))
					.fill(colors.positive)
					.stroke(egui::Stroke::NONE)
					.corner_radius(8)
					.min_size(egui::vec2(160.0, 40.0)),
			)
		} else {
			ui.add_enabled_ui(unavailable.is_none(), |ui| {
				crate::icons::button(ui, crate::icons::Icon::Phone, 32.0, label)
			})
			.inner
		}
		.on_hover_text(hint)
		.on_disabled_hover_text(hint);
		if response.clicked()
			&& let Some(command) = state.start_call_with_mute(
				channel,
				!guild && !incoming,
				self.voice_muted,
				self.voice_deafened,
			) {
			commands.push(command);
		}
		response
	}

	pub(super) fn voice_settings(&mut self, ui: &mut egui::Ui, demo: bool, active: bool) {
		let trigger =
			crate::icons::button(ui, crate::icons::Icon::Headphones, 32.0, "Output settings");
		self.voice_settings_popup(&trigger, demo, active, false);
	}

	/// Input or output half of Discord's voice popout, matching the chevron that opened it.
	fn voice_settings_popup(
		&mut self,
		trigger: &egui::Response,
		demo: bool,
		active: bool,
		input: bool,
	) {
		let id = trigger.id.with("voice-settings-open");
		let mut open = trigger
			.ctx
			.data_mut(|data| *data.get_temp_mut_or_default::<bool>(id));

		if trigger.clicked() {
			open = !open;
		}
		// Device dropdowns use egui's popup memory; keep the parent independently open.
		let close_behavior = if egui::Popup::is_any_open(&trigger.ctx) {
			egui::PopupCloseBehavior::IgnoreClicks
		} else {
			egui::PopupCloseBehavior::CloseOnClickOutside
		};
		egui::Popup::menu(trigger)
			.open_bool(&mut open)
			.style(|_: &mut egui::Style| {})
			.width(340.0)
			.close_behavior(close_behavior)
			.frame(
				egui::Frame::popup(&trigger.ctx.style_of(trigger.ctx.theme()))
					.inner_margin(16)
					.corner_radius(12),
			)
			.show(|ui| {
				ui.set_width(308.0);
				ui.spacing_mut().item_spacing.y = 10.0;
				ui.label(design::semibold(
					ui,
					if input { "Input" } else { "Output" },
					18.0,
				));
				egui::ScrollArea::vertical()
					.max_height((ui.ctx().content_rect().height() - 180.0).clamp(180.0, 460.0))
					.show(ui, |ui| self.voice_popup_content(ui, demo, active, input));
				ui.separator();
				if ui
					.add_sized(
						[ui.available_width(), 32.0],
						egui::Button::new("All voice settings"),
					)
					.clicked()
				{
					self.open_voice_settings();
					ui.close();
				}
			});
		trigger.ctx.data_mut(|data| data.insert_temp(id, open));
	}

	/// One half of the voice popout: the device, its level and the toggles that belong to it.
	fn voice_popup_content(&mut self, ui: &mut egui::Ui, demo: bool, active: bool, input: bool) {
		let colors = design::palette(ui);
		ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
		if demo || !self.voice_available {
			ui.label(
				RichText::new(if demo {
					"Offline preview · microphone and speakers are off."
				} else {
					"Install a voice-enabled build to use these controls."
				})
				.size(13.0)
				.color(colors.muted),
			);
		}
		ui.add_enabled_ui(!demo && self.voice_available, |ui| {
			// Both settings surfaces share this path. Queue discovery once, without opening streams.
			if ui.is_enabled() && self.voice_device_status.is_empty() {
				self.voice_device_status = "Looking for audio devices...";
				self.voice_refresh_devices = true;
				ui.ctx().request_repaint();
			}
			let label = ui
				.horizontal(|ui| {
					crate::icons::inline(
						ui,
						if input {
							crate::icons::Icon::Microphone
						} else {
							crate::icons::Icon::Headphones
						},
						18.0,
						colors.muted,
					);
					ui.label(design::medium(
						ui,
						if input { "Microphone" } else { "Speakers" },
						15.0,
					))
				})
				.inner;
			if input {
				device_combo(ui, "voice-input", &self.voice_inputs, &mut self.voice_input)
					.labelled_by(label.id);
				gain_slider(ui, &mut self.voice_gain.input_percent, "Microphone gain");
			} else {
				device_combo(
					ui,
					"voice-output",
					&self.voice_outputs,
					&mut self.voice_output,
				)
				.labelled_by(label.id);
				gain_slider(ui, &mut self.voice_gain.output_percent, "Speaker volume");
			}
			ui.horizontal_wrapped(|ui| {
				if ui.small_button("Refresh devices").clicked() {
					self.voice_refresh_devices = true;
				}
				if ui.small_button("Reset levels").clicked() {
					self.voice_gain = crate::VoiceGain::default();
				}
			});
			ui.separator();
			if input {
				design::switch(
					ui,
					"Noise suppression",
					Some("Reduce keyboard noise, breathing and fans."),
					&mut self.voice_noise_suppression,
				);
				design::switch(
					ui,
					"Push to talk",
					Some("Hold your configured shortcut when you want to speak."),
					&mut self.voice_push_to_talk,
				);
			} else {
				ui.label(
					RichText::new(
						"Deafen turns off incoming audio and mutes your microphone with it.",
					)
					.size(12.0)
					.color(colors.muted),
				);
			}
		});
		if self.voice_microphone_unavailable {
			ui.label(
				RichText::new(
					"Microphone unavailable · choose another input. You are still connected.",
				)
				.size(12.0)
				.color(colors.warning),
			);
		}
		if !self.voice_device_status.is_empty() {
			ui.label(
				RichText::new(self.voice_device_status)
					.size(12.0)
					.color(colors.muted),
			);
		}
		if active
			&& !input && let Some(code) = &self.voice_privacy_code
		{
			egui::CollapsingHeader::new("Voice privacy code").show(ui, |ui| {
				ui.add(
					egui::Label::new(RichText::new(code).monospace())
						.selectable(true)
						.wrap(),
				);
			});
		}
	}

	pub(super) fn voice_settings_content(
		&mut self,
		ui: &mut egui::Ui,
		demo: bool,
		active: bool,
		compact: bool,
	) {
		let colors = design::palette(ui);
		ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
		ui.spacing_mut().item_spacing.y = if compact { 6.0 } else { 12.0 };
		if demo || !self.voice_available {
			ui.label(
				RichText::new(if demo {
					"Offline preview · microphone and speakers are off."
				} else {
					"Install a voice-enabled build to use these controls."
				})
				.size(13.0)
				.color(colors.muted),
			);
		}
		ui.add_enabled_ui(!demo && self.voice_available, |ui| {
			if compact {
				self.voice_audio_controls(ui);
				egui::CollapsingHeader::new("Voice processing & input mode")
					.show(ui, |ui| self.voice_processing_controls(ui));
			} else {
				ui.label(design::eyebrow(ui, "Devices & levels", colors.muted));
				design::card(ui, |ui| self.voice_audio_controls(ui));
				ui.add_space(8.0);
				ui.label(design::eyebrow(ui, "Voice processing", colors.muted));
				design::card(ui, |ui| self.voice_processing_controls(ui));
			}
		});
		if !compact {
			crate::keybinds::show_voice(
				ui,
				&mut self.keybinds,
				&mut self.keybind_capture,
				self.global_keybind_status,
			);
		}
		ui.add_space(8.0);
		ui.label(design::eyebrow(ui, "Camera", colors.muted));
		design::card(ui, |ui| self.camera_settings_content(ui, demo));
		if active && let Some(code) = &self.voice_privacy_code {
			egui::CollapsingHeader::new("Voice privacy code").show(ui, |ui| {
				ui.add(
					egui::Label::new(RichText::new(code).monospace())
						.selectable(true)
						.wrap(),
				);
				ui.label(
					RichText::new(
						"Compare with the other participants. This code changes with the encrypted call group.",
					)
					.size(12.0)
					.color(colors.muted),
				);
			});
		}
		if !compact {
			ui.label(
				RichText::new(
					"Audio preferences are saved on this device. Your microphone starts only after you join a secured call.",
				)
				.size(12.0)
				.color(colors.muted),
			);
		}
	}

	fn camera_settings_content(&mut self, ui: &mut egui::Ui, demo: bool) {
		let colors = design::palette(ui);
		ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
		if !demo && !cfg!(target_os = "windows") {
			ui.label("Camera selection is currently available on Windows. Other supported platforms use the default camera.");
			return;
		}
		if demo && self.voice_cameras.is_empty() {
			self.voice_cameras = vec![
				(
					"synthetic-integrated".into(),
					"Integrated Camera (preview)".into(),
				),
				("synthetic-usb".into(), "USB Camera (preview)".into()),
			];
		}
		if ui.is_enabled()
			&& !demo && self.voice_camera_device_status.is_empty()
			&& !self.voice_camera_devices_loading
		{
			self.voice_camera_device_status = "Looking for cameras...";
			self.voice_refresh_cameras = true;
			ui.ctx().request_repaint();
		}
		let label = ui.label(design::medium(ui, "Camera device", 15.0));
		device_combo(
			ui,
			"voice-camera",
			&self.voice_cameras,
			&mut self.voice_camera_device,
		)
		.labelled_by(label.id);
		if ui
			.add_enabled(
				!demo && !self.voice_camera_devices_loading,
				egui::Button::new("Refresh cameras").small(),
			)
			.clicked()
		{
			self.voice_refresh_cameras = true;
		}
		if !self.voice_camera_device_status.is_empty() && !demo {
			ui.label(
				RichText::new(self.voice_camera_device_status)
					.size(12.0)
					.color(colors.muted),
			);
		}
		ui.label(
			RichText::new(if demo {
				"Offline preview. These cameras are synthetic; capture is off."
			} else {
				"Changing devices stops your camera and takes effect the next time you turn it on."
			})
			.size(12.0)
			.color(colors.muted),
		);
	}

	fn camera_settings_popup(&mut self, trigger: &egui::Response, demo: bool) {
		let id = trigger.id.with("camera-settings-open");
		let mut open = trigger
			.ctx
			.data_mut(|data| *data.get_temp_mut_or_default::<bool>(id));
		if trigger.clicked() {
			open = !open;
		}
		let close_behavior = if egui::Popup::is_any_open(&trigger.ctx) {
			egui::PopupCloseBehavior::IgnoreClicks
		} else {
			egui::PopupCloseBehavior::CloseOnClickOutside
		};
		egui::Popup::menu(trigger)
			.open_bool(&mut open)
			.style(|_: &mut egui::Style| {})
			.width(300.0)
			.close_behavior(close_behavior)
			.show(|ui| self.camera_settings_content(ui, demo));
		trigger.ctx.data_mut(|data| data.insert_temp(id, open));
	}

	fn voice_audio_controls(&mut self, ui: &mut egui::Ui) {
		// Both settings surfaces use this path. Queue discovery once, without opening streams.
		if ui.is_enabled() && self.voice_device_status.is_empty() {
			self.voice_device_status = "Looking for audio devices...";
			self.voice_refresh_devices = true;
			ui.ctx().request_repaint();
		}
		let colors = design::palette(ui);
		let mut device = |ui: &mut egui::Ui, input: bool| {
			let label = ui
				.horizontal(|ui| {
					crate::icons::inline(
						ui,
						if input {
							crate::icons::Icon::Microphone
						} else {
							crate::icons::Icon::Headphones
						},
						18.0,
						colors.muted,
					);
					ui.label(design::medium(
						ui,
						if input { "Microphone" } else { "Speakers" },
						15.0,
					))
				})
				.inner;
			if input {
				device_combo(ui, "voice-input", &self.voice_inputs, &mut self.voice_input)
					.labelled_by(label.id);
			} else {
				device_combo(
					ui,
					"voice-output",
					&self.voice_outputs,
					&mut self.voice_output,
				)
				.labelled_by(label.id);
			}
		};
		if ui.available_width() >= 480.0 {
			ui.columns(2, |columns| {
				device(&mut columns[0], true);
				device(&mut columns[1], false);
			});
		} else {
			device(ui, true);
			device(ui, false);
		}
		gain_controls(ui, &mut self.voice_gain);
		ui.horizontal_wrapped(|ui| {
			if ui.small_button("Refresh devices").clicked() {
				self.voice_refresh_devices = true;
			}
			if ui.small_button("Reset levels").clicked() {
				self.voice_gain = crate::VoiceGain::default();
			}
		});
		if self.voice_microphone_unavailable {
			ui.label(
				RichText::new(
					"Microphone unavailable · choose another input. You are still connected.",
				)
				.size(12.0)
				.color(colors.warning),
			);
		}
		if !self.voice_device_status.is_empty() {
			ui.label(
				RichText::new(self.voice_device_status)
					.size(12.0)
					.color(colors.muted),
			);
		}
	}

	fn voice_processing_controls(&mut self, ui: &mut egui::Ui) {
		let colors = design::palette(ui);
		design::switch(
			ui,
			"Noise suppression",
			Some("Reduce keyboard noise, breathing and fans."),
			&mut self.voice_noise_suppression,
		)
		.on_hover_text(
			"Reduces background sounds locally. Strong wind or distorted audio may still get through.",
		);
		ui.separator();
		design::switch(
			ui,
			"Push to talk",
			Some("Hold your configured shortcut when you want to speak."),
			&mut self.voice_push_to_talk,
		)
		.on_hover_text("Mute and deafen always take priority.");
		ui.separator();
		ui.horizontal_wrapped(|ui| {
			ui.label(
				RichText::new("Echo cancellation")
					.size(13.0)
					.color(colors.muted),
			);
			ui.label(design::medium(ui, "Always on", 13.0));
		});
	}

	/// Whether the local mute/deafen controls may emit commands for the active call.
	fn controls_enabled(&self, state: &State) -> bool {
		self.voice_available
			&& !state.demo
			&& state
				.voice
				.active
				.as_ref()
				.is_some_and(|call| call.phase != Phase::Failed)
	}

	/// Mute or deafen toggle: red slashed glyph while active, like Discord's user area.
	pub(super) fn mute_toggle(
		&mut self,
		ui: &mut egui::Ui,
		state: &mut State,
		commands: &mut Vec<Command>,
		deafen: bool,
		size: f32,
	) -> egui::Response {
		let colors = design::palette(ui);
		let Some(call) = state.voice.active.as_ref() else {
			let active = if deafen {
				self.voice_deafened
			} else {
				self.voice_muted
			};
			let label = match (deafen, active) {
				(true, true) => "Undeafen",
				(true, false) => "Deafen",
				(false, true) => "Unmute",
				(false, false) => "Mute",
			};
			let (rect, response) =
				ui.allocate_exact_size(egui::Vec2::splat(size), egui::Sense::click());
			if response.hovered() || response.has_focus() {
				ui.painter().rect_filled(rect, 6, colors.hover);
			}
			crate::icons::paint(
				ui.painter(),
				match (deafen, active) {
					(true, true) => crate::icons::Icon::HeadphonesSlash,
					(true, false) => crate::icons::Icon::Headphones,
					(false, true) => crate::icons::Icon::MicrophoneSlash,
					(false, false) => crate::icons::Icon::Microphone,
				},
				rect.shrink(size * 0.2),
				if active { colors.danger } else { colors.muted },
			);
			response.widget_info(|| {
				egui::WidgetInfo::selected(egui::WidgetType::Button, true, active, label)
			});
			if response.clicked() {
				if deafen {
					self.voice_deafened = !active;
				} else {
					self.voice_muted = !active;
				}
			}
			return response.on_hover_text(format!("{label}; applies to your next call."));
		};
		let channel = call.channel;
		let can_speak = state.can_speak(channel);
		let (mut muted, mut deafened) = (self.voice_muted || !can_speak, self.voice_deafened);
		let active = if deafen { deafened } else { muted };
		let enabled =
			(self.controls_enabled(state) || state.demo) && (deafen || can_speak || state.demo);
		let label = match (deafen, active) {
			(true, true) => "Undeafen",
			(true, false) => "Deafen",
			(false, true) => "Unmute",
			(false, false) => "Mute",
		};
		let response = ui
			.add_enabled_ui(enabled, |ui| {
				let (rect, response) =
					ui.allocate_exact_size(egui::Vec2::splat(size), egui::Sense::click());
				if response.hovered() || response.has_focus() {
					ui.painter().rect_filled(rect, 6, colors.hover);
				}
				let icon = match (deafen, active) {
					(true, true) => crate::icons::Icon::HeadphonesSlash,
					(true, false) => crate::icons::Icon::Headphones,
					(false, true) => crate::icons::Icon::MicrophoneSlash,
					(false, false) => crate::icons::Icon::Microphone,
				};
				let color = if !enabled {
					colors.muted.gamma_multiply(0.5)
				} else if active {
					colors.danger
				} else if response.hovered() || response.has_focus() {
					colors.text_strong
				} else {
					colors.muted
				};
				crate::icons::paint(ui.painter(), icon, rect.shrink(size * 0.2), color);
				response.widget_info(|| {
					egui::WidgetInfo::selected(egui::WidgetType::Button, enabled, active, label)
				});
				response
			})
			.inner
			.on_hover_text(if enabled {
				label
			} else if !can_speak && !deafen {
				"Speaking is unavailable in this channel."
			} else {
				"Controls are unavailable in this build or preview."
			});
		if response.clicked() {
			if deafen {
				deafened = !deafened;
			} else {
				muted = !muted;
			}
			self.voice_muted = muted;
			self.voice_deafened = deafened;
			if let Some(command) = state.set_call_mute(muted, deafened) {
				commands.push(command);
			}
		}
		response
	}

	/// Discord's call control bar: one media pill and the red hang-up button.
	fn call_controls(
		&mut self,
		ui: &mut egui::Ui,
		state: &mut State,
		channel: Id,
		commands: &mut Vec<Command>,
	) {
		let colors = design::palette(ui);
		let Some(call) = state.voice.active.as_ref() else {
			return;
		};
		let phase = call.phase;
		let camera = call.camera;
		let can_camera = self.voice_camera_available
			&& state.can_camera(channel)
			&& matches!(phase, Phase::Connected | Phase::Waiting);
		let can_speak = state.can_speak(channel);
		let (mut muted, mut deafened) = (self.voice_muted || !can_speak, self.voice_deafened);
		let controls = self.controls_enabled(state);
		let voice_toggles = controls || state.demo;
		let focused = self.voice_focus.is_some();
		let pill_width = MEDIA_PILL + if focused { 48.0 } else { 0.0 };
		let width = pill_width + BAR_GAP + HANG_UP;
		let mut camera_clicked = false;
		let mut mute_clicked = false;
		let mut deafen_clicked = false;
		let mut leave = false;
		ui.horizontal(|ui| {
			ui.spacing_mut().item_spacing.x = BAR_GAP;
			ui.add_space(((ui.available_width() - width) * 0.5).max(0.0));
			pill(ui, pill_width, |ui| {
				let mic = control(
					ui,
					if muted {
						crate::icons::Icon::MicrophoneSlash
					} else {
						crate::icons::Icon::Microphone
					},
					48.0,
					voice_toggles && (can_speak || state.demo),
					if muted { colors.danger } else { STAGE_TEXT },
					if muted { "Unmute" } else { "Mute" },
					if !can_speak {
						"Speaking is unavailable in this channel."
					} else if muted {
						"Turn on microphone"
					} else {
						"Turn off microphone"
					},
				);
				mute_clicked = mic.clicked();
				let settings = control(
					ui,
					crate::icons::Icon::ChevronDown,
					28.0,
					true,
					STAGE_TEXT,
					"Voice settings",
					"Microphone and speaker settings",
				);
				self.voice_settings_popup(&settings, state.demo, true, true);
				deafen_clicked = control(
					ui,
					if deafened {
						crate::icons::Icon::HeadphonesSlash
					} else {
						crate::icons::Icon::Headphones
					},
					48.0,
					voice_toggles,
					if deafened { colors.danger } else { STAGE_TEXT },
					if deafened { "Undeafen" } else { "Deafen" },
					if deafened {
						"Turn on incoming audio"
					} else {
						"Turn off incoming audio"
					},
				)
				.clicked();
				camera_clicked = control(
					ui,
					if camera {
						crate::icons::Icon::Video
					} else {
						crate::icons::Icon::VideoSlash
					},
					48.0,
					controls && (camera || can_camera),
					if camera { colors.positive } else { STAGE_TEXT },
					if camera {
						"Turn off camera"
					} else {
						"Turn on camera"
					},
					if camera {
						"Stop sharing your camera"
					} else if state.demo {
						"Camera is off in the offline preview"
					} else if !cfg!(any(
						target_os = "macos",
						target_os = "windows",
						target_os = "linux"
					)) {
						"Camera capture is unavailable on this platform"
					} else if !self.voice_camera_available {
						"Camera requires H264 support from the voice server"
					} else if !state.can_camera(channel) {
						"Camera is unavailable with current channel permissions"
					} else {
						"Share your selected camera with this call"
					},
				)
				.clicked();
				let camera_settings = control(
					ui,
					crate::icons::Icon::ChevronDown,
					28.0,
					true,
					STAGE_TEXT,
					"Camera settings",
					"Choose a camera",
				);
				self.camera_settings_popup(&camera_settings, state.demo);
				self.screen_share_control(ui, state);
				if focused {
					let shown = self.voice_focus_participants;
					if control(
						ui,
						crate::icons::Icon::People,
						48.0,
						true,
						if shown { colors.accent } else { STAGE_TEXT },
						if shown {
							"Hide participants"
						} else {
							"Show participants"
						},
						if shown {
							"Hide the participant strip under the enlarged video"
						} else {
							"Show the other participants under the enlarged video"
						},
					)
					.clicked()
					{
						self.voice_focus_participants = !shown;
					}
				}
			});
			let hang_up = {
				let (rect, response) = ui
					.allocate_exact_size(egui::vec2(HANG_UP, CONTROL_HEIGHT), egui::Sense::click());
				let enabled = !state.demo;
				let fill = if !enabled {
					colors.danger.gamma_multiply(0.45)
				} else if response.hovered() || response.has_focus() {
					colors.danger.gamma_multiply(0.85)
				} else {
					colors.danger
				};
				ui.painter().rect_filled(rect, 12, fill);
				crate::icons::paint(
					ui.painter(),
					crate::icons::Icon::HangUp,
					egui::Rect::from_center_size(rect.center(), egui::Vec2::splat(22.0)),
					egui::Color32::WHITE,
				);
				let label = if phase == Phase::Failed {
					"Dismiss call"
				} else {
					"Disconnect"
				};
				response.widget_info(|| {
					egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, label)
				});
				response.on_hover_text(if enabled {
					label
				} else {
					"Leaving is unavailable in the offline preview."
				})
			};
			leave = hang_up.clicked() && !state.demo;
		});
		if camera_clicked && let Some(command) = state.set_call_camera(!camera) {
			self.voice_camera_status = "";
			commands.push(command);
		}
		if mute_clicked {
			muted = !muted;
		}
		if deafen_clicked {
			deafened = !deafened;
		}
		if mute_clicked || deafen_clicked {
			self.voice_muted = muted;
			self.voice_deafened = deafened;
		}
		if (mute_clicked || deafen_clicked)
			&& let Some(command) = state.set_call_mute(muted, deafened)
		{
			commands.push(command);
		}
		if leave && let Some(command) = state.leave_call() {
			commands.push(command);
		}
	}

	fn screen_share_control(&mut self, ui: &mut egui::Ui, state: &State) {
		let enabled = self.screen.busy
			|| state.demo
			|| (self.screen.supported
				&& state.voice.active.as_ref().is_some_and(|call| {
					matches!(call.phase, Phase::Connected | Phase::Waiting)
						&& state.can_stream(call.channel)
				}));
		let label = if self.screen.busy {
			"Stop sharing"
		} else {
			"Share your screen"
		};
		let color = if self.screen.busy {
			design::palette(ui).accent
		} else {
			STAGE_TEXT
		};
		if control(
			ui,
			crate::icons::Icon::ScreenShare,
			48.0,
			enabled,
			color,
			label,
			if enabled {
				self.screen
					.capture_status
					.unwrap_or(if self.screen.status.is_empty() {
						label
					} else {
						self.screen.status
					})
			} else {
				"Screen sharing requires a connected call and video permission on a supported desktop."
			},
		)
		.clicked()
		{
			self.screen.launch(state);
		}
	}

	/// DM call stage above the conversation, plus the incoming-call banner.
	pub(super) fn call_bar(
		&mut self,
		ui: &mut egui::Ui,
		state: &mut State,
		commands: &mut Vec<Command>,
	) {
		let colors = design::palette(ui);
		let selected = state.selected;
		if let Some(channel) = state
			.voice
			.active
			.as_ref()
			.filter(|call| call.guild.is_none() && Some(call.channel) == selected)
			.map(|call| call.channel)
		{
			let height = if self.stage_shows_video(state, channel) || state.is_group_dm(channel) {
				(ui.available_height() * 0.74).clamp(320.0, 900.0)
			} else {
				(ui.available_height() * 0.42).clamp(240.0, 340.0)
			};
			egui::Panel::top("dm-call")
				.exact_size(height)
				.show_separator_line(false)
				.frame(egui::Frame::new().fill(STAGE_FILL))
				.show(ui, |ui| {
					let rect = ui.max_rect();
					let notices = self.stage_notices(state, channel, true);
					let mut notice_ui = ui.new_child(
						egui::UiBuilder::new()
							.max_rect(rect.shrink(STAGE_MARGIN))
							.layout(egui::Layout::top_down(egui::Align::Min)),
					);
					stage_notices(&mut notice_ui, &notices);
					call_failure(
						&mut notice_ui,
						state.voice.active.as_ref().and_then(|c| c.error),
						STAGE_TEXT,
					);
					let body = egui::Rect::from_min_max(
						egui::pos2(rect.left() + STAGE_MARGIN, notice_ui.cursor().top() + 8.0),
						egui::pos2(
							rect.right() - STAGE_MARGIN,
							rect.bottom() - CONTROL_HEIGHT - 2.0 * STAGE_MARGIN,
						),
					);
					let mut body_ui = ui.new_child(
						egui::UiBuilder::new()
							.max_rect(body)
							.layout(egui::Layout::top_down(egui::Align::Min)),
					);
					self.participant_tiles(
						&mut body_ui,
						state,
						channel,
						&stage_participants(state, channel),
						true,
					);
					let bar = egui::Rect::from_min_max(
						egui::pos2(rect.left(), rect.bottom() - CONTROL_HEIGHT - STAGE_MARGIN),
						egui::pos2(rect.right(), rect.bottom() - STAGE_MARGIN),
					);
					let mut bar_ui = ui.new_child(
						egui::UiBuilder::new()
							.max_rect(bar)
							.layout(egui::Layout::left_to_right(egui::Align::Center)),
					);
					self.call_controls(&mut bar_ui, state, channel, commands);
				});
			self.apply_watch_request(state);
		}
		let existing = selected.filter(|channel| {
			state.voice.has_dm_call(*channel)
				&& state.can_view(*channel)
				&& state
					.voice
					.active
					.as_ref()
					.is_none_or(|call| call.channel != *channel)
		});
		if let Some(channel) = state.voice.incoming.or(existing) {
			let incoming = state.voice.incoming == Some(channel);
			let conversation = state.channel(channel).cloned();
			let name = state
				.channels
				.iter()
				.find(|c| c.id == channel)
				.map_or("Direct message", |c| c.name.as_str())
				.to_owned();
			let unavailable = self.call_unavailable(state, channel);
			egui::Panel::top("dm-incoming")
				.show_separator_line(false)
				.frame(
					egui::Frame::new()
						.fill(colors.raised)
						.inner_margin(egui::Margin::symmetric(16, 10)),
				)
				.show(ui, |ui| {
					ui.horizontal(|ui| {
						ui.spacing_mut().item_spacing.x = 12.0;
						match conversation.as_ref() {
							Some(group) if group.kind == 3 => {
								self.avatars.show_group(ui, group, 40.0, state.demo);
							}
							Some(dm) if dm.kind == 1 && !dm.recipients.is_empty() => {
								let user = &dm.recipients[0];
								self.avatars.show(ui, user, 40.0, state.demo);
							}
							_ => {
								design::avatar(ui, &name, 40.0);
							}
						}
						ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
							ui.spacing_mut().item_spacing.x = 8.0;
							if incoming {
								let decline = round_action(
									ui,
									crate::icons::Icon::HangUp,
									colors.danger,
									!state.demo,
									"Decline",
								);
								if decline.clicked()
									&& let Some(command) = state.decline_call()
								{
									commands.push(command);
								}
							}
							let answer = if incoming {
								round_action(
									ui,
									crate::icons::Icon::Phone,
									colors.positive,
									unavailable.is_none(),
									"Answer",
								)
							} else {
								ui.add_enabled(
									unavailable.is_none(),
									egui::Button::new(
										design::medium(ui, "Join call", 13.0)
											.color(egui::Color32::WHITE),
									)
									.fill(colors.positive)
									.min_size(egui::vec2(84.0, 36.0)),
								)
							}
							.on_disabled_hover_text(unavailable.unwrap_or(""));
							if answer.clicked()
								&& let Some(command) = state.start_call_with_mute(
									channel,
									false,
									self.voice_muted,
									self.voice_deafened,
								) {
								commands.push(command);
							}
							ui.with_layout(
								egui::Layout::left_to_right(egui::Align::Center),
								|ui| {
									ui.vertical(|ui| {
										ui.spacing_mut().item_spacing.y = 2.0;
										ui.add(
											egui::Label::new(
												design::semibold(ui, &name, 15.0)
													.color(colors.text_strong),
											)
											.truncate(),
										);
										ui.label(
											RichText::new(if incoming {
												unavailable.unwrap_or("Incoming call…")
											} else if !state.gateway_connected {
												"Reconnect to refresh call"
											} else {
												"Call in progress"
											})
											.size(13.0)
											.color(colors.muted),
										);
									});
								},
							);
						});
					});
				});
		}
	}

	/// Call details drawn inside the account card while connected: Discord's "Voice
	/// Connected" header, then a row of quick actions above the identity row.
	pub(super) fn voice_card_section(
		&mut self,
		ui: &mut egui::Ui,
		state: &mut State,
		commands: &mut Vec<Command>,
	) {
		let Some(call) = state.voice.active.as_ref() else {
			return;
		};
		let colors = design::palette(ui);
		let phase = call.phase;
		let channel_id = call.channel;
		let camera = call.camera;
		let connected = matches!(phase, Phase::Connected | Phase::Waiting);
		let error = call.error;
		let channel = state
			.channel(call.channel)
			.map_or("Direct message", |c| c.name.as_str())
			.to_owned();
		let guild = call
			.guild
			.and_then(|id| state.guild(id))
			.map(|g| g.name.clone());
		let detail = match guild {
			Some(guild) => format!("{channel} / {guild}"),
			None => channel,
		};
		let title = if state.demo && phase != Phase::Failed {
			"Voice preview"
		} else if phase == Phase::Failed {
			"Call failed"
		} else if connected {
			"Voice Connected"
		} else {
			"Connecting…"
		};
		let color = if phase == Phase::Failed {
			colors.danger
		} else if connected || state.demo {
			colors.positive
		} else {
			colors.warning
		};
		egui::Frame::new()
			.inner_margin(egui::Margin {
				left: 8,
				right: 8,
				top: 8,
				bottom: 8,
			})
			.show(ui, |ui| {
				ui.set_width(ui.available_width());
				ui.spacing_mut().item_spacing.y = 8.0;
				if self.voice_microphone_unavailable {
					ui.label(
						RichText::new(
							"Microphone unavailable · still connected. Choose another input in Audio settings.",
						)
						.size(12.0)
						.color(colors.warning),
					);
				}
				ui.horizontal(|ui| {
					ui.spacing_mut().item_spacing.x = 10.0;
					// Square status tile like Discord's, tinted with the connection colour.
					let (tile, _) =
						ui.allocate_exact_size(egui::Vec2::splat(40.0), egui::Sense::hover());
					ui.painter().rect_filled(tile, 8, colors.base);
					crate::icons::paint(
						ui.painter(),
						crate::icons::Icon::InCall,
						tile.shrink(10.0),
						color,
					);
					ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
						ui.spacing_mut().item_spacing.x = 2.0;
						let leave = ui
							.add_enabled_ui(!state.demo, |ui| {
								crate::icons::button(
									ui,
									crate::icons::Icon::HangUp,
									32.0,
									if phase == Phase::Failed {
										"Dismiss call"
									} else {
										"Disconnect"
									},
								)
							})
							.inner;
						if leave.clicked()
							&& let Some(command) = state.leave_call()
						{
							commands.push(command);
						}
						ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
							ui.vertical(|ui| {
								ui.spacing_mut().item_spacing.y = 0.0;
								ui.add(
									egui::Label::new(
										design::semibold(ui, title, 15.0).color(color),
									)
									.truncate()
									.selectable(false),
								);
								ui.add(
									egui::Label::new(
										RichText::new(detail).size(12.0).color(colors.muted),
									)
									.truncate()
									.selectable(false),
								);
							});
						});
					});
				});
				call_failure(ui, error, colors.text_strong);
				let controls = self.controls_enabled(state);
				let can_camera = self.voice_camera_available
					&& state.can_camera(channel_id)
					&& matches!(phase, Phase::Connected | Phase::Waiting);
				let can_share = self.screen.busy
					|| state.demo || (self.screen.supported
					&& matches!(phase, Phase::Connected | Phase::Waiting)
					&& state.can_stream(channel_id));
				let processing = !state.demo && self.voice_available;
				let mut camera_clicked = false;
				let mut share_clicked = false;
				ui.horizontal(|ui| {
					ui.spacing_mut().item_spacing.x = 8.0;
					let width = ((ui.available_width() - 3.0 * 8.0 - 24.0) / 3.0).max(24.0);
					camera_clicked = card_action(
						ui,
						width,
						if camera {
							crate::icons::Icon::Video
						} else {
							crate::icons::Icon::VideoSlash
						},
						controls && (camera || can_camera),
						camera,
						if camera {
							"Turn off camera"
						} else {
							"Turn on camera"
						},
						if camera {
							"Stop sharing your camera"
						} else if state.demo {
							"Camera is off in the offline preview"
						} else if !self.voice_camera_available {
							"Camera requires H264 support from the voice server"
						} else if !state.can_camera(channel_id) {
							"Camera is unavailable with current channel permissions"
						} else {
							"Share your selected camera with this call"
						},
					)
					.clicked();
					let camera_settings = crate::icons::button(
						ui,
						crate::icons::Icon::ChevronDown,
						24.0,
						"Camera settings",
					);
					self.camera_settings_popup(&camera_settings, state.demo);
					share_clicked = card_action(
						ui,
						width,
						crate::icons::Icon::ScreenShare,
						can_share,
						self.screen.busy,
						if self.screen.busy {
							"Stop sharing"
						} else {
							"Share your screen"
						},
						if can_share {
							if let Some(status) = self.screen.capture_status {
								status
							} else if self.screen.busy {
								"Stop sharing your screen"
							} else {
								"Share a screen or window"
							}
						} else {
							"Screen sharing requires a connected call and video permission on a supported desktop."
						},
					)
					.clicked();
					if card_action(
						ui,
						width,
						crate::icons::Icon::Soundboard,
						processing,
						self.voice_noise_suppression,
						if self.voice_noise_suppression {
							"Turn off noise suppression"
						} else {
							"Turn on noise suppression"
						},
						if processing {
							"Noise suppression reduces keyboard noise, breathing and fans locally."
						} else {
							"Noise suppression is unavailable in this build or preview."
						},
					)
					.clicked()
					{
						self.voice_noise_suppression = !self.voice_noise_suppression;
					}
				});
				if camera_clicked && let Some(command) = state.set_call_camera(!camera) {
					self.voice_camera_status = "";
					commands.push(command);
				}
				if share_clicked {
					self.screen.launch(state);
				}
			});
		if connected && ui.is_rect_visible(ui.max_rect()) {
			ui.ctx()
				.request_repaint_after(std::time::Duration::from_secs(1));
		}
	}

	/// Mic or headphones toggle followed by Discord's small chevron opening voice settings.
	pub(super) fn mute_toggle_with_settings(
		&mut self,
		ui: &mut egui::Ui,
		state: &mut State,
		commands: &mut Vec<Command>,
		deafen: bool,
	) {
		let chevron = crate::icons::button(
			ui,
			crate::icons::Icon::ChevronDown,
			20.0,
			if deafen {
				"Output settings"
			} else {
				"Input settings"
			},
		);
		self.voice_settings_popup(&chevron, state.demo, state.voice.active.is_some(), !deafen);
		self.mute_toggle(ui, state, commands, deafen, 32.0);
	}
}

/// Quick action in the in-call account card: filled cell, accent glyph while the feature is on.
fn card_action(
	ui: &mut egui::Ui,
	width: f32,
	icon: crate::icons::Icon,
	enabled: bool,
	active: bool,
	label: &str,
	hint: &str,
) -> egui::Response {
	let colors = design::palette(ui);
	let (rect, response) = ui.allocate_exact_size(
		egui::vec2(width, 40.0),
		if enabled {
			egui::Sense::click()
		} else {
			egui::Sense::hover()
		},
	);
	let fill = if enabled && (response.hovered() || response.has_focus()) {
		colors.selected
	} else {
		colors.hover
	};
	ui.painter().rect_filled(rect, 8, fill);
	let color = if !enabled {
		colors.muted.gamma_multiply(0.5)
	} else if active {
		colors.accent
	} else {
		colors.text_strong
	};
	crate::icons::paint(
		ui.painter(),
		icon,
		egui::Rect::from_center_size(rect.center(), egui::Vec2::splat(20.0)),
		color,
	);
	response.widget_info(|| {
		egui::WidgetInfo::selected(egui::WidgetType::Button, enabled, active, label)
	});
	response.on_hover_text(hint)
}

/// Discord's call stage is black in every appearance; pills and tiles sit on it in fixed greys.
const STAGE_FILL: egui::Color32 = egui::Color32::BLACK;
const TILE_FILL: egui::Color32 = egui::Color32::from_rgb(0x2b, 0x2d, 0x31);
const PILL_FILL: egui::Color32 = egui::Color32::from_rgb(0x1e, 0x1f, 0x22);
const STAGE_TEXT: egui::Color32 = egui::Color32::from_rgb(0xdb, 0xde, 0xe1);
const STAGE_MUTED: egui::Color32 = egui::Color32::from_rgb(0x9a, 0x9b, 0xa1);
const STAGE_MARGIN: f32 = 16.0;
const TILE_GAP: f32 = 8.0;
/// Height shared by every control, pill and the hang-up button in the call bar.
const CONTROL_HEIGHT: f32 = 48.0;
/// Mic, settings chevron, deafen, camera and screen share sit in one pill.
const MEDIA_PILL: f32 = 248.0;
const HANG_UP: f32 = 64.0;
const BAR_GAP: f32 = 12.0;

/// Green ring like Discord's speaking indicator; the accessible label still names the state.
/// Which stage tile is enlarged; cleared automatically when it stops showing video.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StageFocus {
	LocalScreen,
	Stream(Id),
	Participant(Id),
}

enum Tile<'a> {
	LocalScreen,
	Stream(Id),
	Participant(&'a RosterEntry),
}
impl Tile<'_> {
	fn focus(&self) -> StageFocus {
		match self {
			Self::LocalScreen => StageFocus::LocalScreen,
			Self::Stream(streamer) => StageFocus::Stream(*streamer),
			Self::Participant(entry) => StageFocus::Participant(entry.participant.user),
		}
	}
	/// Stable interaction identity, so a tile keeps its hover while the layout changes.
	fn key(&self) -> (u8, u64) {
		match self.focus() {
			StageFocus::LocalScreen => (0, 0),
			StageFocus::Stream(user) => (1, user.0),
			StageFocus::Participant(user) => (2, user.0),
		}
	}
}

/// Column count and tile size that make the tiles largest inside `area`, at 16:9.
fn best_fit(count: usize, area: egui::Vec2, cap: f32) -> (usize, egui::Vec2) {
	let mut best = (1, egui::Vec2::ZERO);
	for columns in 1..=count.max(1) {
		let rows = count.div_ceil(columns);
		let width = (area.x - TILE_GAP * (columns - 1) as f32) / columns as f32;
		let height = (area.y - TILE_GAP * (rows - 1) as f32) / rows as f32;
		if width <= 0.0 || height <= 0.0 {
			continue;
		}
		let width = width.min(height * 16.0 / 9.0).min(cap);
		// Prefer side-by-side tiles when the size cap makes multiple layouts tie.
		if width >= best.1.x {
			best = (columns, egui::vec2(width, width * 9.0 / 16.0));
		}
	}
	if best.1.x <= 0.0 {
		best = (count.max(1), egui::Vec2::ZERO);
	}
	best
}

/// The largest centred rectangle of the image's aspect ratio inside `rect`.
fn fit_rect(rect: egui::Rect, image: egui::Vec2) -> egui::Rect {
	if image.x <= 0.0 || image.y <= 0.0 {
		return rect;
	}
	let size = image * (rect.width() / image.x).min(rect.height() / image.y);
	egui::Rect::from_center_size(rect.center(), size)
}

/// Fill the tile edge to edge, cropping the overflow (Discord's camera framing).
fn cover_image(
	ui: &mut egui::Ui,
	rect: egui::Rect,
	id: egui::TextureId,
	image: egui::Vec2,
	mirror: bool,
) {
	if image.x <= 0.0 || image.y <= 0.0 {
		return;
	}
	let scale = (rect.width() / image.x).max(rect.height() / image.y);
	let shown = egui::vec2(
		(rect.width() / (image.x * scale)).min(1.0),
		(rect.height() / (image.y * scale)).min(1.0),
	);
	let mut uv = egui::Rect::from_center_size(egui::pos2(0.5, 0.5), shown);
	if mirror {
		std::mem::swap(&mut uv.min.x, &mut uv.max.x);
	}
	ui.put(
		rect,
		egui::Image::from_texture((id, rect.size()))
			.uv(uv)
			.corner_radius(8),
	);
}

/// Bottom-left translucent name plate, optionally with the mute glyph.
fn name_badge(ui: &mut egui::Ui, rect: egui::Rect, name: &str, icon: Option<crate::icons::Icon>) {
	let font = egui::FontId::new(13.0, design::medium_family(ui.ctx()));
	let icon_width = if icon.is_some() { 20.0 } else { 0.0 };
	// One line, truncated with an ellipsis; the tile hover text carries the full name.
	let mut job = egui::text::LayoutJob::simple_singleline(name.to_owned(), font, STAGE_TEXT);
	job.wrap.max_width = (rect.width() * 0.6 - icon_width).max(20.0);
	job.wrap.max_rows = 1;
	job.wrap.break_anywhere = true;
	let galley = ui.painter().layout_job(job);
	let badge = egui::Rect::from_min_size(
		rect.left_bottom() + egui::vec2(8.0, -8.0 - 24.0),
		egui::vec2(galley.size().x + 16.0 + icon_width, 24.0),
	);
	ui.painter()
		.rect_filled(badge, 6, egui::Color32::from_black_alpha(160));
	ui.painter().galley(
		egui::pos2(badge.left() + 8.0, badge.center().y - galley.size().y * 0.5),
		galley,
		STAGE_TEXT,
	);
	if let Some(icon) = icon {
		crate::icons::paint(
			ui.painter(),
			icon,
			egui::Rect::from_center_size(
				egui::pos2(badge.right() - 14.0, badge.center().y),
				egui::Vec2::splat(14.0),
			),
			design::palette(ui).danger,
		);
	}
}

/// Bottom-right pill action on a tile, sized to its label with a hover fill.
fn tile_button(
	ui: &mut egui::Ui,
	rect: egui::Rect,
	label: &str,
	fill: egui::Color32,
	hover_fill: egui::Color32,
	hint: &str,
) -> egui::Response {
	let font = egui::FontId::new(12.0, design::medium_family(ui.ctx()));
	let galley = ui
		.painter()
		.layout_no_wrap(label.to_owned(), font, egui::Color32::WHITE);
	let size = egui::vec2(galley.size().x + 20.0, 26.0);
	let button = egui::Rect::from_min_size(
		egui::pos2(rect.right() - 8.0 - size.x, rect.top() + 8.0),
		size,
	);
	let response = ui.allocate_rect(button, egui::Sense::click());
	let fill = if response.hovered() || response.has_focus() {
		hover_fill
	} else {
		fill
	};
	ui.painter().rect_filled(button, 6, fill);
	ui.painter().galley(
		button.center() - galley.size() * 0.5,
		galley,
		egui::Color32::WHITE,
	);
	response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label));
	response.on_hover_text(hint)
}

fn speaking_avatar(ui: &egui::Ui, avatar: &egui::Response, name: &str) {
	let colors = design::palette(ui);
	ui.painter().circle_stroke(
		avatar.rect.center(),
		avatar.rect.width() * 0.5 + 2.0,
		egui::Stroke::new(2.0, colors.positive),
	);
	let label = format!("{name} · Speaking");
	avatar.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Image, true, &label));
	avatar.clone().on_hover_text(label);
}

fn call_failure(ui: &mut egui::Ui, error: Option<&str>, color: egui::Color32) {
	let Some(error) = error else { return };
	ui.horizontal_top(|ui| {
		ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
			if crate::icons::button(ui, crate::icons::Icon::Copy, 28.0, "Copy failure reason")
				.clicked()
			{
				ui.ctx()
					.copy_text(format!("Serein call failed\nReason: {error}"));
			}
			ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| {
				ui.add(
					egui::Label::new(RichText::new(error).size(12.0).color(color))
						.wrap()
						.selectable(true),
				);
			});
		});
	});
}

fn stage_notices(ui: &mut egui::Ui, notices: &[(String, bool)]) {
	ui.spacing_mut().item_spacing.y = 2.0;
	for (text, strong) in notices {
		ui.add(
			egui::Label::new(
				RichText::new(text)
					.size(if *strong { 13.0 } else { 12.0 })
					.color(if *strong { STAGE_TEXT } else { STAGE_MUTED }),
			)
			.truncate()
			.selectable(false),
		);
	}
	if !notices.is_empty() {
		ui.add_space(6.0);
	}
	ui.spacing_mut().item_spacing.y = 8.0;
}

/// Rounded dark group holding several call controls. Every pill is exactly `CONTROL_HEIGHT`
/// tall and `width` wide so the groups and the hang-up button share one baseline.
fn pill<R>(ui: &mut egui::Ui, width: f32, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
	let (rect, _) = ui.allocate_exact_size(egui::vec2(width, CONTROL_HEIGHT), egui::Sense::hover());
	ui.painter().rect_filled(rect, 12, PILL_FILL);
	ui.painter().rect_stroke(
		rect,
		12,
		egui::Stroke::new(1.0, egui::Color32::from_white_alpha(18)),
		egui::StrokeKind::Inside,
	);
	let mut inner = ui.new_child(
		egui::UiBuilder::new()
			.max_rect(rect)
			.layout(egui::Layout::left_to_right(egui::Align::Center)),
	);
	inner.spacing_mut().item_spacing.x = 0.0;
	add(&mut inner)
}

/// One control inside a pill; disabled controls stay visible but inert, like Discord's.
fn control(
	ui: &mut egui::Ui,
	icon: crate::icons::Icon,
	width: f32,
	enabled: bool,
	color: egui::Color32,
	label: &str,
	hint: &str,
) -> egui::Response {
	let (rect, response) = ui.allocate_exact_size(
		egui::vec2(width, CONTROL_HEIGHT),
		if enabled {
			egui::Sense::click()
		} else {
			egui::Sense::hover()
		},
	);
	if enabled && (response.hovered() || response.has_focus()) {
		ui.painter()
			.rect_filled(rect.shrink(4.0), 8, egui::Color32::from_white_alpha(28));
	}
	let size = if width < 40.0 { 14.0 } else { 22.0 };
	crate::icons::paint(
		ui.painter(),
		icon,
		egui::Rect::from_center_size(rect.center(), egui::Vec2::splat(size)),
		if enabled {
			color
		} else {
			STAGE_MUTED.gamma_multiply(0.45)
		},
	);
	response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, label));
	response.on_hover_text(hint)
}

/// Circular filled action (answer/decline) used by the incoming-call banner.
fn round_action(
	ui: &mut egui::Ui,
	icon: crate::icons::Icon,
	fill: egui::Color32,
	enabled: bool,
	label: &str,
) -> egui::Response {
	let (rect, response) = ui.allocate_exact_size(
		egui::Vec2::splat(40.0),
		if enabled {
			egui::Sense::click()
		} else {
			egui::Sense::hover()
		},
	);
	let fill = if !enabled {
		fill.gamma_multiply(0.45)
	} else if response.hovered() || response.has_focus() {
		fill.gamma_multiply(0.85)
	} else {
		fill
	};
	ui.painter().circle_filled(rect.center(), 20.0, fill);
	crate::icons::paint(
		ui.painter(),
		icon,
		egui::Rect::from_center_size(rect.center(), egui::Vec2::splat(20.0)),
		egui::Color32::WHITE,
	);
	response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, label));
	response.on_hover_text(label)
}

/// Resolve the display name and user for a roster entry from the entry, member list or self.
/// Include the local call participant before its gateway roster update arrives.
fn stage_participants(state: &State, channel: Id) -> Vec<RosterEntry> {
	let call = state
		.voice
		.active
		.as_ref()
		.filter(|call| call.channel == channel);
	let mut entries: Vec<_> = if let Some(call) = call.filter(|call| call.guild.is_none()) {
		call.participants
			.iter()
			.map(|participant| RosterEntry {
				guild: Id(0),
				channel,
				participant: *participant,
				member: None,
			})
			.collect()
	} else {
		state
			.voice
			.roster
			.iter()
			.filter(|entry| entry.channel == channel)
			.cloned()
			.collect()
	};
	if let Some(call) = call.filter(|call| call.phase != Phase::Failed)
		&& let Some(user) = &state.user
	{
		if let Some(index) = entries
			.iter()
			.position(|entry| entry.participant.user == user.id)
		{
			entries.swap(0, index);
		} else {
			entries.insert(
				0,
				RosterEntry {
					guild: call.guild.unwrap_or(Id(0)),
					channel,
					participant: Participant {
						user: user.id,
						muted: call.muted,
						deafened: call.deafened,
						server_muted: call.server_muted,
						server_deafened: call.server_deafened,
						video: call.camera,
						streaming: false,
					},
					member: None,
				},
			);
		}
	}
	entries
}

fn resolve_member<'a>(
	state: &'a State,
	entry: &'a RosterEntry,
) -> (Option<&'a model::User>, &'a str) {
	let member = entry.member.as_ref().or_else(|| {
		state
			.members
			.as_ref()
			.filter(|list| list.guild == Some(entry.guild))
			.and_then(|list| {
				list.rows
					.iter()
					.flatten()
					.find(|m| m.user.id == entry.participant.user)
			})
	});
	let user = member
		.map(|m| &m.user)
		.or_else(|| participant_user(state, entry.channel, entry.participant.user));
	let name = member
		.and_then(|m| m.nick.as_deref())
		.or_else(|| user.map(|u| u.name.as_str()))
		.unwrap_or("Participant");
	(user, name)
}

/// Find a call participant's user from self, DM recipients or the roster.
fn participant_user(state: &State, channel: Id, user: Id) -> Option<&model::User> {
	state
		.user
		.as_ref()
		.filter(|u| u.id == user)
		.or_else(|| {
			state
				.channels
				.iter()
				.find(|c| c.id == channel)
				.and_then(|c| c.recipients.iter().find(|u| u.id == user))
		})
		.or_else(|| {
			state
				.voice
				.roster
				.iter()
				.find(|e| e.channel == channel && e.participant.user == user)
				.and_then(|e| e.member.as_ref())
				.map(|m| &m.user)
		})
}

/// Labelled percentage slider shared by the voice popout and the settings page.
fn gain_slider(ui: &mut egui::Ui, value: &mut u16, title: &str) -> egui::Response {
	ui.scope(|ui| {
		let colors = design::palette(ui);
		let label = ui.label(RichText::new(title).size(13.0).color(colors.muted));
		ui.spacing_mut().slider_width = (ui.available_width() - 64.0).max(80.0);
		ui.visuals_mut().widgets.inactive.bg_fill = colors.border;
		ui.add(
			egui::Slider::new(value, 0..=200)
				.suffix("%")
				.step_by(1.0)
				.trailing_fill(true)
				.handle_shape(egui::style::HandleShape::Circle),
		)
		.labelled_by(label.id)
	})
	.inner
}

fn gain_controls(ui: &mut egui::Ui, gain: &mut crate::VoiceGain) -> [egui::Response; 2] {
	let slider = gain_slider;
	let responses = if ui.available_width() >= 480.0 {
		ui.columns(2, |columns| {
			[
				slider(&mut columns[0], &mut gain.input_percent, "Microphone gain"),
				slider(&mut columns[1], &mut gain.output_percent, "Speaker volume"),
			]
		})
	} else {
		[
			slider(ui, &mut gain.input_percent, "Microphone gain"),
			slider(ui, &mut gain.output_percent, "Speaker volume"),
		]
	};
	ui.label(
		RichText::new("100% is the original level. Higher levels may distort.")
			.size(12.0)
			.color(design::palette(ui).muted),
	);
	responses
}

fn elapsed_label(call: &client_core::voice::Call) -> Option<String> {
	if !matches!(call.phase, Phase::Waiting | Phase::Connected) {
		return None;
	}
	let seconds = call.connected_at?.elapsed().as_secs();
	Some(format!(
		"{:02}:{:02}:{:02}",
		seconds / 3600,
		seconds / 60 % 60,
		seconds % 60
	))
}

fn status_icon(ui: &mut egui::Ui, icon: crate::icons::Icon, color: egui::Color32, label: &str) {
	let (rect, response) = ui.allocate_exact_size(egui::vec2(20.0, 20.0), egui::Sense::hover());
	crate::icons::paint(ui.painter(), icon, rect.shrink(1.0), color);
	response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, label));
	response.on_hover_text(label);
}

/// Discord's LIVE pill, sized for the channel list row rather than a stage tile.
fn live_badge(ui: &mut egui::Ui) {
	let (rect, response) = ui.allocate_exact_size(egui::vec2(32.0, 16.0), egui::Sense::hover());
	let colors = design::palette(ui);
	ui.painter().rect_filled(rect, 4, colors.danger);
	ui.painter().text(
		rect.center(),
		egui::Align2::CENTER_CENTER,
		"LIVE",
		egui::FontId::new(9.0, design::medium_family(ui.ctx())),
		egui::Color32::WHITE,
	);
	response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, "Live"));
	response.on_hover_text("Streaming");
}

fn device_combo(
	ui: &mut egui::Ui,
	id: &str,
	devices: &[(String, String)],
	selected: &mut Option<String>,
) -> egui::Response {
	let label = match selected.as_ref() {
		None => "System default",
		Some(id) => devices
			.iter()
			.find(|(key, _)| key == id)
			.map_or("Device unavailable", |(_, label)| label.as_str()),
	};
	egui::ComboBox::from_id_salt(id)
		.selected_text(label)
		.width(ui.available_width())
		.truncate()
		.height(220.0)
		.show_ui(ui, |ui| {
			ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
			ui.selectable_value(selected, None, "System default");
			for (id, label) in devices.iter().take(32) {
				ui.selectable_value(selected, Some(id.clone()), label)
					.on_hover_text(label);
			}
		})
		.response
		.on_hover_text(label)
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn local_mutes_zero_one_speaker_and_keep_their_stored_volume() {
		let mut view = MessagingUi::default();
		view.set_voice_user_volume_overrides(&[(7, 150)]);
		assert!(!view.voice_user_locally_muted(Id(7)));
		view.set_voice_user_locally_muted(Id(7), true);
		assert_eq!(view.voice_user_mutes(), [7]);
		assert!(view.voice_user_volumes().contains(&(7, 0)));
		// Muting is device-local and never rewrites the volume chosen for that speaker.
		assert_eq!(view.voice_user_volume_overrides(), vec![(7, 150)]);
		view.set_voice_user_locally_muted(Id(9), true);
		assert!(view.voice_user_volumes().contains(&(9, 0)));
		view.set_voice_user_locally_muted(Id(7), false);
		assert!(view.voice_user_volumes().contains(&(7, 150)));
		assert_eq!(view.voice_user_mutes(), [9]);
		view.set_voice_user_mutes(&(0..200).collect::<Vec<u64>>());
		assert_eq!(view.voice_user_mutes().len(), MAX_USER_MUTES);
		assert!(!view.voice_user_mutes().contains(&0));
	}

	#[test]
	fn camera_settings_bound_layout_and_only_request_discovery_once() {
		let mut disabled = MessagingUi::default();
		egui::Context::default()
			.run_ui(Default::default(), |ui| {
				ui.add_enabled_ui(false, |ui| disabled.camera_settings_content(ui, false));
			})
			.drop_without_applying_deltas();
		assert!(!disabled.voice_refresh_cameras);
		assert!(disabled.voice_camera_device_status.is_empty());
		for demo in [false, true] {
			for theme in [egui::Theme::Dark, egui::Theme::Light] {
				let ctx = egui::Context::default();
				ctx.set_theme(theme);
				let mut messaging = MessagingUi {
					voice_cameras: vec![(
						"synthetic-camera".into(),
						"Long synthetic camera name ".repeat(20),
					)],
					voice_camera_device: Some("synthetic-camera".into()),
					..Default::default()
				};
				for width in [240.0, 640.0] {
					for frame in 0..2 {
						let mut output = ctx.run_ui(
							egui::RawInput {
								screen_rect: Some(egui::Rect::from_min_size(
									egui::Pos2::ZERO,
									egui::vec2(width, 600.0),
								)),
								..Default::default()
							},
							|ui| {
								let available = ui.available_width();
								let content =
									ui.scope(|ui| messaging.camera_settings_content(ui, demo));
								assert!(content.response.rect.width() <= available + 1.0);
							},
						);
						output.textures_delta.clear();
						assert_eq!(
							messaging.voice_camera_device.as_deref(),
							Some("synthetic-camera")
						);
						assert_eq!(
							messaging.voice_refresh_cameras,
							!demo && cfg!(target_os = "windows") && width == 240.0 && frame == 0
						);
						assert!(messaging.voice_camera_preview.is_none());
						messaging.voice_refresh_cameras = false;
					}
				}
			}
		}
	}

	#[test]
	fn camera_popup_selects_second_device_without_starting_capture() {
		fn labels(shape: &egui::Shape, out: &mut Vec<(String, egui::Rect)>) {
			match shape {
				egui::Shape::Text(text) => out.push((
					text.galley.job.text.clone(),
					text.galley.rect.translate(text.pos.to_vec2()),
				)),
				egui::Shape::Vec(shapes) => shapes.iter().for_each(|shape| labels(shape, out)),
				_ => {}
			}
		}
		let ctx = egui::Context::default();
		let frame = |messaging: &mut MessagingUi, events: Vec<egui::Event>| {
			let mut output = ctx.run_ui(
				egui::RawInput {
					screen_rect: Some(egui::Rect::from_min_size(
						egui::Pos2::ZERO,
						egui::vec2(640.0, 600.0),
					)),
					events,
					..Default::default()
				},
				|ui| {
					let trigger = ui.button("Choose camera");
					messaging.camera_settings_popup(&trigger, true);
				},
			);
			output.textures_delta.clear();
			let mut text = vec![];
			for shape in output.shapes {
				labels(&shape.shape, &mut text);
			}
			text
		};
		let click = |messaging: &mut MessagingUi, pos: egui::Pos2| {
			for pressed in [true, false] {
				frame(
					messaging,
					vec![
						egui::Event::PointerMoved(pos),
						egui::Event::PointerButton {
							pos,
							button: egui::PointerButton::Primary,
							pressed,
							modifiers: egui::Modifiers::NONE,
						},
					],
				);
			}
		};
		let mut messaging = MessagingUi::default();
		for label in ["Choose camera", "System default", "USB Camera (preview)"] {
			frame(&mut messaging, vec![]);
			let text = frame(&mut messaging, vec![]);
			let pos = text
				.iter()
				.find(|(text, _)| text == label)
				.unwrap_or_else(|| panic!("Missing control: {label}"))
				.1
				.center();
			click(&mut messaging, pos);
		}
		assert_eq!(
			messaging.voice_camera_device.as_deref(),
			Some("synthetic-usb")
		);
		assert!(!messaging.voice_refresh_cameras);
		assert!(messaging.voice_camera_preview.is_none());
		assert!(
			frame(&mut messaging, vec![])
				.iter()
				.any(|(text, _)| text == "Camera device"),
			"Selecting a device keeps the parent camera popup open"
		);
	}

	#[test]
	fn solo_call_stages_show_both_local_previews_without_a_roster() {
		fn textures(shape: &egui::Shape, ids: &mut Vec<egui::TextureId>) {
			match shape {
				egui::Shape::Mesh(mesh) => ids.push(mesh.texture_id),
				egui::Shape::Rect(rect) => {
					if let Some(brush) = &rect.brush {
						ids.push(brush.fill_texture_id);
					}
				}
				egui::Shape::Vec(shapes) => shapes.iter().for_each(|shape| textures(shape, ids)),
				_ => {}
			}
		}
		for guild in [false, true] {
			for width in [320.0, 900.0] {
				for dark in [false, true] {
					let mut state = if guild {
						test_support::voice_demo_state()
					} else {
						test_support::call_demo_state()
					};
					state.voice.roster.clear();
					let call = state.voice.active.as_mut().unwrap();
					call.participants.clear();
					call.phase = Phase::Waiting;
					call.camera = true;
					let (channel, request) = (call.channel, call.request);
					let ctx = egui::Context::default();
					ctx.set_visuals(if dark {
						egui::Visuals::dark()
					} else {
						egui::Visuals::light()
					});
					let mut messaging = MessagingUi::default();
					let camera = ctx.load_texture(
						"synthetic-camera",
						egui::ColorImage::filled([4, 3], egui::Color32::RED),
						Default::default(),
					);
					let screen = ctx.load_texture(
						"synthetic-screen",
						egui::ColorImage::filled([16, 9], egui::Color32::BLUE),
						Default::default(),
					);
					let expected = [camera.id(), screen.id()];
					messaging.voice_camera_preview = Some(camera);
					messaging.screen.preview = Some(screen);
					messaging.screen.busy = true;
					messaging.screen.context = Some((state.generation, channel, request));
					assert_eq!(stage_participants(&state, channel).len(), 1);
					let mut commands = vec![];
					for phase in [Phase::Waiting, Phase::Connected, Phase::Failed] {
						state.voice.active.as_mut().unwrap().phase = phase;
						let mut output = ctx.run_ui(
							egui::RawInput {
								screen_rect: Some(egui::Rect::from_min_size(
									egui::Pos2::ZERO,
									egui::vec2(width, 800.0),
								)),
								..Default::default()
							},
							|ui| {
								if guild {
									messaging.voice_channel(ui, &mut state, channel, &mut commands);
								} else {
									messaging.call_bar(ui, &mut state, &mut commands);
								}
							},
						);
						let mut rendered = vec![];
						for shape in &output.shapes {
							textures(&shape.shape, &mut rendered);
						}
						output.textures_delta.clear();
						for id in expected {
							assert_eq!(
								rendered.contains(&id),
								phase != Phase::Failed,
								"guild={guild}, width={width}, phase={phase:?}, texture={id:?}"
							);
						}
					}
					assert!(
						commands.is_empty(),
						"Synthetic previews must never start media"
					);
					state.voice.active = None;
					assert!(stage_participants(&state, channel).is_empty());
				}
			}
		}
	}

	#[test]
	fn existing_dm_call_banner_joins_without_ringing_and_disables_unavailable_actions() {
		fn frame(
			ctx: &egui::Context,
			messaging: &mut MessagingUi,
			state: &mut State,
			width: f32,
			events: Vec<egui::Event>,
		) -> (Vec<(String, egui::Rect)>, Vec<Command>) {
			fn labels(shape: &egui::Shape, out: &mut Vec<(String, egui::Rect)>) {
				match shape {
					egui::Shape::Text(text) => out.push((
						text.galley.job.text.clone(),
						text.galley.rect.translate(text.pos.to_vec2()),
					)),
					egui::Shape::Vec(shapes) => shapes.iter().for_each(|shape| labels(shape, out)),
					_ => {}
				}
			}
			let mut commands = vec![];
			let mut output = ctx.run_ui(
				egui::RawInput {
					screen_rect: Some(egui::Rect::from_min_size(
						egui::Pos2::ZERO,
						egui::vec2(width, 480.0),
					)),
					events,
					..Default::default()
				},
				|ui| messaging.call_bar(ui, state, &mut commands),
			);
			output.textures_delta.clear();
			let mut text = vec![];
			for shape in output.shapes {
				labels(&shape.shape, &mut text);
			}
			(text, commands)
		}
		for width in [320.0, 900.0] {
			for dark in [false, true] {
				for mode in 0..5 {
					let mut state = test_support::existing_call_demo_state();
					state.demo = mode == 1;
					state.gateway_connected = mode != 2;
					if mode == 4 {
						assert!(state.start_call(Id(25), false).is_some());
					}
					state
						.channels
						.iter_mut()
						.find(|c| c.id == Id(22))
						.unwrap()
						.name = "A long synthetic caller name ".repeat(8);
					let mut messaging = MessagingUi {
						voice_available: mode != 3,
						..Default::default()
					};
					let ctx = egui::Context::default();
					ctx.set_visuals(if dark {
						egui::Visuals::dark()
					} else {
						egui::Visuals::light()
					});
					frame(&ctx, &mut messaging, &mut state, width, vec![]);
					let (text, commands) = frame(&ctx, &mut messaging, &mut state, width, vec![]);
					assert!(
						commands.is_empty(),
						"Showing an ongoing call must never join"
					);
					let join = text
						.iter()
						.find(|(label, _)| label == "Join call")
						.expect("Visible Join call button")
						.1;
					assert!(
						join.left() >= 0.0 && join.right() <= width,
						"Join must fit narrow layouts"
					);
					assert!(text.iter().any(|(label, _)| label
						== if mode == 2 {
							"Reconnect to refresh call"
						} else {
							"Call in progress"
						}));
					let mut sent = vec![];
					for pressed in [true, false] {
						let (_, commands) = frame(
							&ctx,
							&mut messaging,
							&mut state,
							width,
							vec![
								egui::Event::PointerMoved(join.center()),
								egui::Event::PointerButton {
									pos: join.center(),
									button: egui::PointerButton::Primary,
									pressed,
									modifiers: egui::Modifiers::NONE,
								},
							],
						);
						sent.extend(commands);
					}
					if mode == 0 {
						assert!(matches!(
							sent.as_slice(),
							[Command::Voice(client_core::voice::Command::Join {
								channel: Id(22),
								ring: false,
								..
							})]
						));
					} else {
						assert!(
							sent.is_empty(),
							"Demo, offline, voice-unavailable and busy states cannot join"
						);
					}
					state.apply_voice(client_core::voice::Event::Deleted { channel: Id(22) });
					let (text, _) = frame(&ctx, &mut messaging, &mut state, width, vec![]);
					assert!(
						!text
							.iter()
							.any(|(label, _)| label == "Join call" || label == "Call in progress")
					);
				}
			}
		}
	}

	#[test]
	fn voice_popup_keeps_device_selection_open_and_demo_controls_inert() {
		fn labels(shape: &egui::Shape, out: &mut Vec<(String, egui::Rect)>) {
			match shape {
				egui::Shape::Text(text) => out.push((
					text.galley.job.text.clone(),
					text.galley.rect.translate(text.pos.to_vec2()),
				)),
				egui::Shape::Vec(shapes) => shapes.iter().for_each(|shape| labels(shape, out)),
				_ => {}
			}
		}
		fn frame(
			ctx: &egui::Context,
			messaging: &mut MessagingUi,
			demo: bool,
			events: Vec<egui::Event>,
		) -> Vec<(String, egui::Rect)> {
			let mut output = ctx.run_ui(
				egui::RawInput {
					screen_rect: Some(egui::Rect::from_min_size(
						egui::Pos2::ZERO,
						egui::vec2(800.0, 800.0),
					)),
					events,
					..Default::default()
				},
				|ui| {
					let trigger = ui.button("Open voice");
					messaging.voice_settings_popup(&trigger, demo, false, true);
				},
			);
			output.textures_delta.clear();
			let mut text = vec![];
			for shape in output.shapes {
				labels(&shape.shape, &mut text);
			}
			text
		}
		fn click(ctx: &egui::Context, messaging: &mut MessagingUi, demo: bool, pos: egui::Pos2) {
			for pressed in [true, false] {
				frame(
					ctx,
					messaging,
					demo,
					vec![
						egui::Event::PointerMoved(pos),
						egui::Event::PointerButton {
							pos,
							button: egui::PointerButton::Primary,
							pressed,
							modifiers: egui::Modifiers::NONE,
						},
					],
				);
			}
		}
		let position = |text: &[(String, egui::Rect)], label: &str| {
			text.iter()
				.find(|(value, _)| value == label)
				.unwrap_or_else(|| panic!("Missing visible control: {label}"))
				.1
				.center()
		};
		for demo in [false, true] {
			let ctx = egui::Context::default();
			let mut messaging = MessagingUi {
				voice_available: true,
				voice_inputs: vec![("synthetic-input".into(), "Synthetic headset".into())],
				voice_gain: crate::VoiceGain {
					input_percent: 140,
					output_percent: 80,
				},
				..Default::default()
			};
			frame(&ctx, &mut messaging, demo, vec![]);
			let text = frame(&ctx, &mut messaging, demo, vec![]);
			click(&ctx, &mut messaging, demo, position(&text, "Open voice"));
			let text = frame(&ctx, &mut messaging, demo, vec![]);
			click(
				&ctx,
				&mut messaging,
				demo,
				position(&text, "System default"),
			);
			let text = frame(&ctx, &mut messaging, demo, vec![]);
			if demo {
				assert!(
					!egui::Popup::is_any_open(&ctx),
					"Disabled devices must not open a picker"
				);
				click(
					&ctx,
					&mut messaging,
					demo,
					position(&text, "Refresh devices"),
				);
				let text = frame(&ctx, &mut messaging, demo, vec![]);
				click(&ctx, &mut messaging, demo, position(&text, "Reset levels"));
				assert_eq!(messaging.voice_input, None);
				assert!(!messaging.voice_refresh_devices);
				assert_eq!(messaging.voice_gain.input_percent, 140);
				assert_eq!(messaging.voice_gain.output_percent, 80);
			} else {
				assert!(
					egui::Popup::is_any_open(&ctx),
					"The device picker must survive its opening frame"
				);
				click(
					&ctx,
					&mut messaging,
					demo,
					position(&text, "Synthetic headset"),
				);
				assert_eq!(messaging.voice_input.as_deref(), Some("synthetic-input"));
				assert!(!egui::Popup::is_any_open(&ctx));
			}
			let text = frame(&ctx, &mut messaging, demo, vec![]);
			assert!(
				text.iter().any(|(value, _)| value == "All voice settings"),
				"Changing settings must keep the voice popup open"
			);
			click(&ctx, &mut messaging, demo, egui::pos2(760.0, 760.0));
			let text = frame(&ctx, &mut messaging, demo, vec![]);
			assert!(
				!text.iter().any(|(value, _)| value == "All voice settings"),
				"Clicking outside must close the popup"
			);
		}
	}

	#[test]
	fn gain_sliders_accept_keyboard_input_and_reset_with_the_session() {
		let mut messaging = MessagingUi::default();
		assert_eq!(messaging.voice_gain.input_percent, 100);
		assert_eq!(messaging.voice_gain.output_percent, 100);
		let ctx = egui::Context::default();
		let raw = || egui::RawInput {
			screen_rect: Some(egui::Rect::from_min_size(
				egui::Pos2::ZERO,
				egui::vec2(240.0, 260.0),
			)),
			..Default::default()
		};
		ctx.run_ui(raw(), |ui| {
			gain_controls(ui, &mut messaging.voice_gain)[0].request_focus();
		})
		.drop_without_applying_deltas();
		let mut input = raw();
		input.events.push(egui::Event::Key {
			key: egui::Key::ArrowRight,
			physical_key: None,
			pressed: true,
			repeat: false,
			modifiers: egui::Modifiers::NONE,
		});
		ctx.run_ui(input, |ui| {
			let controls = gain_controls(ui, &mut messaging.voice_gain);
			assert!(
				controls
					.iter()
					.all(|r| r.rect.right() <= ui.max_rect().right() + 1.0)
			);
		})
		.drop_without_applying_deltas();
		assert_eq!(messaging.voice_gain.input_percent, 101);
		assert_eq!(messaging.voice_gain.output_percent, 100);
		assert!(
			!messaging.voice_refresh_devices,
			"Gain does not enumerate devices"
		);
		messaging.voice_gain.input_percent = u16::MAX;
		messaging.voice_gain.output_percent = 0;
		ctx.run_ui(raw(), |ui| {
			gain_controls(ui, &mut messaging.voice_gain);
		})
		.drop_without_applying_deltas();
		assert_eq!(messaging.voice_gain.input_percent, 200);
		assert_eq!(messaging.voice_gain.output_percent, 0);
		messaging.clear();
		assert_eq!(messaging.voice_gain.input_percent, 100);
		assert_eq!(messaging.voice_gain.output_percent, 100);
	}

	#[test]
	fn guild_voice_requires_explicit_keyboard_join_and_demo_never_emits_media() {
		let mut state = test_support::demo_state();
		state.demo = false;
		assert!(matches!(
			state.select(Id(25)),
			Some(client_core::Command::History {
				channel: Id(25),
				..
			})
		));
		assert_eq!(state.selected, Some(Id(25)));
		assert!(state.voice.active.is_none());
		let mut messaging = MessagingUi {
			voice_available: true,
			..Default::default()
		};
		let context = egui::Context::default();
		let mut commands = vec![];
		context
			.run_ui(Default::default(), |ui| {
				messaging
					.call_button(ui, &mut state, Id(25), &mut commands)
					.request_focus();
			})
			.drop_without_applying_deltas();
		assert!(commands.is_empty());
		assert!(state.voice.active.is_none());
		let enter = egui::RawInput {
			events: vec![egui::Event::Key {
				key: egui::Key::Enter,
				physical_key: None,
				pressed: true,
				repeat: false,
				modifiers: egui::Modifiers::NONE,
			}],
			..Default::default()
		};
		context
			.run_ui(enter, |ui| {
				messaging.call_button(ui, &mut state, Id(25), &mut commands);
			})
			.drop_without_applying_deltas();
		assert!(matches!(
			commands.as_slice(),
			[Command::Voice(client_core::voice::Command::Join {
				channel: Id(25),
				ring: false,
				..
			})]
		));
		let call = state.voice.active.as_mut().unwrap();
		assert!(elapsed_label(call).is_none());
		call.connected_at = Some(std::time::Instant::now() - std::time::Duration::from_secs(3663));
		call.phase = Phase::Waiting;
		assert_eq!(elapsed_label(call).as_deref(), Some("01:01:03"));
		call.phase = Phase::Failed;
		assert!(elapsed_label(call).is_none());
		state.demo = true;
		for width in [640.0, 1120.0] {
			let mut output = context.run_ui(
				egui::RawInput {
					screen_rect: Some(egui::Rect::from_min_size(
						egui::Pos2::ZERO,
						egui::vec2(width, 480.0),
					)),
					..Default::default()
				},
				|ui| {
					assert!(
						messaging
							.show(ui, &mut state)
							.iter()
							.all(|c| !matches!(c, Command::Voice(_) | Command::History { .. }))
					);
				},
			);
			output.textures_delta.clear();
		}
		assert!(messaging.take_avatar_requests().is_empty());
		state.voice.active = None;
		assert!(messaging.call_unavailable(&state, Id(25)).is_some());
		state.demo = false;
		messaging.voice_available = false;
		assert!(
			messaging
				.call_unavailable(&state, Id(25))
				.unwrap()
				.contains("Voice is unavailable")
		);
	}

	#[test]
	fn voice_roster_marks_streaming_participants_live() {
		let mut state = test_support::demo_state();
		state.voice.roster = vec![RosterEntry {
			guild: Id(10),
			channel: Id(25),
			participant: client_core::voice::Participant {
				user: Id(1),
				muted: false,
				deafened: false,
				server_muted: false,
				server_deafened: false,
				video: false,
				streaming: true,
			},
			member: Some(model::Member {
				user: model::User {
					id: Id(1),
					name: "i play baal".into(),
					avatar: None,
					webhook: false,
					kind: Default::default(),
					discriminator: 0,
					primary_guild: None,
				},
				nick: None,
				roles: vec![],
				status: None,
				custom_status: None,
				activities: vec![],
				clients: model::ClientPlatforms::default(),
			}),
		}];
		let mut messaging = MessagingUi::default();
		let ctx = egui::Context::default();
		let output = ctx.run_ui(
			egui::RawInput {
				screen_rect: Some(egui::Rect::from_min_size(
					egui::Pos2::ZERO,
					egui::vec2(190.0, 120.0),
				)),
				..Default::default()
			},
			|ui| messaging.voice_participant(ui, &state, &state.voice.roster[0]),
		);
		let texts: Vec<_> = output
			.shapes
			.iter()
			.filter_map(|shape| match &shape.shape {
				egui::Shape::Text(text) => Some(text.galley.job.text.as_str()),
				_ => None,
			})
			.collect();
		assert!(
			texts.contains(&"LIVE"),
			"Streamers get a LIVE pill: {texts:?}"
		);
		assert!(
			texts.iter().any(|text| text.contains("i play baal")),
			"The name stays alongside the pill: {texts:?}"
		);
		output.drop_without_applying_deltas();
		state.voice.roster[0].participant.streaming = false;
		let output = ctx.run_ui(
			egui::RawInput {
				screen_rect: Some(egui::Rect::from_min_size(
					egui::Pos2::ZERO,
					egui::vec2(190.0, 120.0),
				)),
				..Default::default()
			},
			|ui| messaging.voice_participant(ui, &state, &state.voice.roster[0]),
		);
		assert!(
			!output.shapes.iter().any(|shape| matches!(
				&shape.shape,
				egui::Shape::Text(text) if text.galley.job.text == "LIVE"
			)),
			"Idle participants keep a plain row"
		);
		output.drop_without_applying_deltas();
	}

	#[test]
	fn voice_roster_preserves_status_space_with_long_names_and_virtualizes() {
		let mut state = State {
			demo: true,
			selected: Some(Id(25)),
			..Default::default()
		};
		state.voice.roster = (1..=64)
			.map(|id| RosterEntry {
				guild: Id(10),
				channel: Id(25),
				participant: client_core::voice::Participant {
					user: Id(id),
					muted: true,
					deafened: true,
					server_muted: false,
					server_deafened: false,
					video: false,
					streaming: false,
				},
				member: Some(model::Member {
					user: model::User {
						id: Id(id),
						name: "Long synthetic participant name ".repeat(5),
						avatar: None,
						webhook: false,
						kind: Default::default(),
						discriminator: 0,
						primary_guild: None,
					},
					nick: None,
					roles: vec![],
					status: None,
					custom_status: None,
					activities: vec![],
					clients: model::ClientPlatforms::default(),
				}),
			})
			.collect();
		let mut messaging = MessagingUi::default();
		let ctx = egui::Context::default();
		for theme in [egui::Theme::Light, egui::Theme::Dark] {
			ctx.set_theme(theme);
			let mut output = ctx.run_ui(
				egui::RawInput {
					screen_rect: Some(egui::Rect::from_min_size(
						egui::Pos2::ZERO,
						egui::vec2(190.0, 320.0),
					)),
					..Default::default()
				},
				|ui| {
					let width = ui.available_width();
					let row = ui.scope(|ui| {
						messaging.voice_participant(ui, &state, &state.voice.roster[0])
					});
					assert!(
						row.response.rect.width() <= width + 1.0,
						"Long names must not displace the mute/deafen icons"
					);
					messaging.voice_channel(ui, &mut state, Id(25), &mut vec![]);
				},
			);
			assert!(
				output.textures_delta.set.len() < 20,
				"Only visible avatars should be loaded"
			);
			output.textures_delta.clear();
		}
		assert!(messaging.take_avatar_requests().is_empty());
	}

	#[test]
	fn viewing_an_incoming_call_never_answers_and_preview_cannot_call() {
		let mut state = State {
			demo: true,
			selected: Some(Id(1)),
			auth: AuthState::Authenticated,
			gateway_connected: true,
			channels: vec![model::Channel {
				last_message: None,
				id: Id(1),
				guild: None,
				parent_id: None,
				position: 0,
				name: "Synthetic DM".into(),
				kind: 1,
				recipients: vec![model::User {
					id: Id(2),
					name: "Synthetic peer".into(),
					avatar: None,
					webhook: false,
					kind: Default::default(),
					discriminator: 0,
					primary_guild: None,
				}],
				member_list_id: None,
				message_count: None,
				icon: None,
			}],
			..Default::default()
		};
		state.voice.incoming = Some(Id(1));
		let mut messaging = MessagingUi {
			voice_available: true,
			..Default::default()
		};
		let context = egui::Context::default();
		let output = context.run_ui(Default::default(), |ui| {
			let commands = messaging.show(ui, &mut state);
			assert!(
				commands
					.iter()
					.all(|command| !matches!(command, Command::Voice(_)))
			);
		});
		output.drop_without_applying_deltas();
		assert!(state.voice.active.is_none());
		assert_eq!(state.voice.incoming, Some(Id(1)));
		assert!(messaging.call_unavailable(&state, Id(1)).is_some());
		state.demo = false;
		let output = context.run_ui(Default::default(), |ui| {
			let commands = messaging.show(ui, &mut state);
			assert!(
				commands
					.iter()
					.all(|command| !matches!(command, Command::Voice(_)))
			);
		});
		output.drop_without_applying_deltas();
		assert!(
			state.voice.active.is_none(),
			"Incoming calls require an explicit answer"
		);
		messaging.voice_available = false;
		assert!(
			messaging
				.call_unavailable(&state, Id(1))
				.unwrap()
				.contains("Voice is unavailable")
		);
		messaging.voice_available = true;
		assert!(messaging.call_unavailable(&state, Id(1)).is_none());
	}
}
