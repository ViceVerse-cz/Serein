//! Voice-only layout: one slim pane listing the servers, their voice channels and who is in
//! them, like Ventrilo. It stands in for the navigation, conversation and member panes while an
//! enabled plugin switches on [`extensions::InterfaceTweak::VoiceOnlyLayout`]. A channel's call
//! window opens beside the pane only once that channel is double-clicked.
use crate::{MessagingUi, design, icons};
use client_core::{
	Command, State,
	voice::{Phase, RosterEntry},
};
use egui::{RichText, collapsing_header::CollapsingState};
use model::Id;
use std::collections::BTreeMap;

/// Channel and participant rows: Ventrilo's slim lines rather than the channel list's rows.
const ROW: f32 = 22.0;
/// Server headers, a little taller than the rows under them.
const SERVER_ROW: f32 = 24.0;
/// The strips above the tree and the call window.
const HEADER: f32 = 28.0;
/// Horizontal step between tree levels, which is also a caret's width.
const INDENT: f32 = 14.0;
/// Speaker and status glyphs.
const GLYPH: f32 = 14.0;

/// The call window beside the tree, opened by double-clicking a voice channel.
#[derive(Clone, Copy)]
pub(super) struct Stage {
	channel: Id,
	/// The call last seen in `channel`; the window closes once that call ends.
	call: Option<u64>,
}

impl MessagingUi {
	/// The client surface between the title bar and the overlays in the voice-only layout: the
	/// voice pane, with a channel's call window beside it once a double click opened it.
	pub(super) fn show_voice_only(
		&mut self,
		ui: &mut egui::Ui,
		state: &mut State,
		commands: &mut Vec<Command>,
	) {
		// The member pane is replaced too, so its subscription closes as when the pane hides.
		if state.members.is_some() {
			commands.push(state.close_members());
		}
		let pane = egui::Frame::new()
			.fill(design::section_surface(
				ui,
				design::window_palette(ui).sidebar,
				design::ImageSection::ChannelList,
			))
			.inner_margin(0);
		if let Some(channel) = self.stage_channel(state) {
			egui::Panel::left("voice-only-pane")
				.resizable(true)
				.show_separator_line(!design::has_window_background(ui))
				.default_size(280.0)
				.size_range(200.0..=480.0)
				.frame(pane)
				.show(ui, |ui| self.voice_only_pane(ui, state, commands));
			let window = egui::Frame::new()
				.fill(design::section_surface(
					ui,
					design::window_palette(ui).chat,
					design::ImageSection::MessageList,
				))
				.inner_margin(0);
			egui::CentralPanel::default()
				.frame(window)
				.show(ui, |ui| self.voice_only_call(ui, state, channel, commands));
		} else {
			egui::CentralPanel::default()
				.frame(pane)
				.show(ui, |ui| self.voice_only_pane(ui, state, commands));
		}
		// Joins and watch requests apply after the tree and the stage, as beside the channel list.
		self.apply_voice_join(state, commands);
		self.apply_stream_preview_watch(state, commands);
	}
	/// The channel whose call window shows. It stays open until its button hides it or the
	/// call seen in that channel ends.
	fn stage_channel(&mut self, state: &State) -> Option<Id> {
		let stage = self.voice_only_stage.as_mut()?;
		let call = state
			.voice
			.active
			.as_ref()
			.filter(|call| call.channel == stage.channel)
			.map(|call| call.request);
		if (call.is_none() && stage.call.is_some()) || state.channel(stage.channel).is_none() {
			self.voice_only_stage = None;
			return None;
		}
		if call.is_some() {
			stage.call = call;
		}
		Some(stage.channel)
	}
	/// The voice pane: the account strip, the voice tree and the call controls.
	fn voice_only_pane(
		&mut self,
		ui: &mut egui::Ui,
		state: &mut State,
		commands: &mut Vec<Command>,
	) {
		egui::Panel::bottom("voice-only-controls")
			.show_separator_line(!design::has_window_background(ui))
			.frame(egui::Frame::new().inner_margin(egui::Margin::symmetric(8, 4)))
			.show(ui, |ui| self.voice_only_controls(ui, state, commands));
		self.voice_only_header(ui, state, commands);
		if let Some(channel) = self.voice_only_tree(ui, state)
			&& let Some(command) = state.select(channel)
		{
			commands.push(command);
		}
	}
	/// The call window: the channel's name and a button hiding the window, over its stage.
	fn voice_only_call(
		&mut self,
		ui: &mut egui::Ui,
		state: &mut State,
		channel: Id,
		commands: &mut Vec<Command>,
	) {
		let mut strip = header_strip(ui);
		let hide = crate::i18n::translate("ventrilo-hide-call");
		if icons::button(&mut strip, icons::Icon::Close, 24.0, &hide).clicked() {
			self.voice_only_stage = None;
			ui.ctx().request_repaint();
		}
		let name = state
			.channel(channel)
			.map_or_else(String::new, |known| known.name.clone());
		strip_title(&mut strip, name);
		self.voice_channel(ui, state, channel, commands);
	}
	/// Settings, the microphone and headphone toggles, and Disconnect during a call: the account
	/// card's controls, sending the same commands.
	fn voice_only_controls(
		&mut self,
		ui: &mut egui::Ui,
		state: &mut State,
		commands: &mut Vec<Command>,
	) {
		ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
			ui.spacing_mut().item_spacing.x = 2.0;
			let settings = icons::button(
				ui,
				icons::Icon::Gear,
				32.0,
				&crate::i18n::translate("user-settings"),
			);
			if settings.clicked() {
				self.settings.open = true;
				egui::Popup::close_all(ui.ctx());
			}
			self.mute_toggle_with_settings(ui, state, commands, true);
			ui.add_space(4.0);
			self.mute_toggle_with_settings(ui, state, commands, false);
			let Some(phase) = state.voice.active.as_ref().map(|call| call.phase) else {
				return;
			};
			ui.add_space(4.0);
			let leave = ui
				.add_enabled_ui(!state.demo, |ui| {
					icons::button(
						ui,
						icons::Icon::HangUp,
						32.0,
						&crate::i18n::translate(if phase == Phase::Failed {
							"voice-voice-card-section-dismiss-call"
						} else {
							"voice-voice-card-section-disconnect"
						}),
					)
				})
				.inner;
			if leave.clicked()
				&& let Some(command) = state.leave_call()
			{
				commands.push(command);
			}
		});
	}
	/// The picked server's voice channels in the channel list's order and the people in them.
	/// A channel is hidden only when VIEW_CHANNEL is explicitly denied; unknown permissions
	/// (a server never opened) still list, and picking that server loads it. Returns a channel
	/// clicked to select.
	fn voice_only_tree(&mut self, ui: &mut egui::Ui, state: &State) -> Option<Id> {
		let current = self.voice_only_current_guild(state);
		let mut channels = Vec::<&model::Channel>::new();
		for channel in &state.channels {
			if current.is_some()
				&& channel.guild == current
				&& crate::categories::voice_lane(channel.kind)
				&& !state.view_channel_denied(channel.id)
			{
				channels.push(channel);
			}
		}
		if channels.is_empty() {
			ui.add_space((ui.available_height() * 0.32).max(24.0));
			ui.vertical_centered(|ui| {
				let empty = RichText::new(crate::i18n::translate("ventrilo-empty"));
				ui.label(empty.color(design::palette(ui).muted));
			});
			return None;
		}
		channels.sort_unstable_by_key(|channel| list_order(state, channel));
		let mut roster = BTreeMap::<Id, Vec<&RosterEntry>>::new();
		for entry in &state.voice.roster {
			if Some(entry.guild) == current {
				roster.entry(entry.channel).or_default().push(entry);
			}
		}
		let mut picked = None;
		self.scroll
			.attach(
				ui,
				"voice-only-tree",
				egui::ScrollArea::vertical().auto_shrink([false, false]),
			)
			.show(ui, |ui| {
				ui.spacing_mut().item_spacing.y = 0.0;
				let Some(guild) = state.guilds.iter().find(|guild| Some(guild.id) == current)
				else {
					return;
				};
				if !guild_row(ui, guild) {
					return;
				}
				for channel in &channels {
					let people = roster.get(&channel.id).map_or(&[][..], Vec::as_slice);
					if self.channel_row(ui, state, channel, !people.is_empty(), &mut picked) {
						for entry in people {
							self.participant_row(ui, state, entry);
						}
					}
				}
			});
		picked
	}
	/// The server the tree lists: the header pick, else the open conversation's server, else
	/// the first server. A pick whose server left the account falls back the same way.
	fn voice_only_current_guild(&self, state: &State) -> Option<Id> {
		self.voice_only_guild
			.filter(|picked| state.guilds.iter().any(|guild| guild.id == *picked))
			.or_else(|| {
				state
					.selected
					.and_then(|channel| state.channel(channel))
					.and_then(|channel| channel.guild)
			})
			.or_else(|| state.guilds.first().map(|guild| guild.id))
	}
	/// A voice channel. A click selects it and shows who is in it; a double click also joins it
	/// and opens its call window, as in Ventrilo; the caret only folds. Returns whether the
	/// people in it are shown.
	fn channel_row(
		&mut self,
		ui: &mut egui::Ui,
		state: &State,
		channel: &model::Channel,
		occupied: bool,
		picked: &mut Option<Id>,
	) -> bool {
		// Global ids keep the fold and the row's state when the call window moves the pane.
		let id = egui::Id::unique(("voice-only-channel", channel.id));
		let mut fold = CollapsingState::load_with_default_open(ui.ctx(), id, true);
		let Some((rect, response)) = tree_row(ui, ROW, id.with("row")) else {
			return fold.is_open();
		};
		let row = rect.shrink2(egui::vec2(4.0, 0.0));
		let caret_left = row.left() + INDENT;
		// Registered over the row, so its clicks fold without selecting or joining.
		let caret = occupied.then(|| {
			let area = row.with_min_x(caret_left).with_max_x(caret_left + INDENT);
			ui.interact(area, id.with("caret"), egui::Sense::click())
		});
		if caret.as_ref().is_some_and(egui::Response::clicked) {
			fold.toggle(ui);
			fold.store(ui.ctx());
		} else if response.clicked() {
			*picked = Some(channel.id);
			if !fold.is_open() {
				fold.set_open(true);
				fold.store(ui.ctx());
			}
		}
		// Keyboard and assistive activation joins too; a pointer single click never does.
		// Double-clicking the channel whose window is open closes it again, as in Ventrilo.
		let keyboard_join = response.has_focus()
			&& ui.input(|input| {
				input.key_pressed(egui::Key::Enter) || input.key_pressed(egui::Key::Space)
			});
		if response.double_clicked() || keyboard_join {
			if self
				.voice_only_stage
				.as_ref()
				.is_some_and(|stage| stage.channel == channel.id)
			{
				self.voice_only_stage = None;
			} else {
				self.voice_join = Some(channel.id);
				self.voice_only_stage = Some(Stage {
					channel: channel.id,
					call: None,
				});
			}
			ui.ctx().request_repaint();
		}
		let open = fold.is_open();
		let connected = state.voice.active.as_ref().is_some_and(|call| {
			call.channel == channel.id && matches!(call.phase, Phase::Connected | Phase::Waiting)
		});
		let selected = state.selected == Some(channel.id);
		let colors = design::palette(ui);
		let caret_hovered = caret
			.as_ref()
			.is_some_and(|button| button.hovered() || button.has_focus());
		let hovered = caret_hovered || response.hovered() || response.has_focus();
		if selected {
			ui.painter().rect_filled(rect, 4, colors.selected);
		} else if hovered {
			paint_hover(ui, rect);
		}
		let color = if selected || hovered || connected {
			colors.text_strong
		} else {
			colors.text
		};
		if occupied {
			paint_caret(ui, row, caret_left, open, color);
		}
		let glyph_left = caret_left + INDENT;
		icons::paint(
			ui.painter(),
			icons::Icon::Speaker,
			glyph(row, glyph_left),
			if connected { colors.positive } else { color },
		);
		let font = egui::FontId::new(13.0, design::medium_family(ui.ctx()));
		let name = row.with_min_x(glyph_left + GLYPH + 4.0);
		paint_text(ui, name, &channel.name, font, color);
		if let Some(caret) = caret {
			caret.widget_info(|| {
				egui::WidgetInfo::labeled(
					egui::Role::Button,
					true,
					format!(
						"{} {}",
						crate::i18n::translate(if open {
							"categories-category-header-collapse"
						} else {
							"categories-category-header-expand"
						}),
						channel.name
					),
				)
			});
		}
		response.widget_info(|| {
			egui::WidgetInfo::selected(
				egui::Role::Button,
				true,
				selected,
				format!(
					"{} {}{}",
					channel.name,
					crate::i18n::translate("voice-voice-channel-button-voice-channel"),
					if connected {
						crate::i18n::translate("voice-voice-channel-button-connected")
					} else {
						String::new()
					}
				),
			)
		});
		if !connected {
			response.on_hover_text(crate::i18n::translate("ventrilo-join-hint"));
		}
		open
	}
	/// One person in a voice channel: the speaker glyph lights up while they talk and turns red
	/// while they are muted. A click opens their profile, as in the channel list.
	fn participant_row(&mut self, ui: &mut egui::Ui, state: &State, entry: &RosterEntry) {
		let user = entry.participant.user;
		let id = egui::Id::unique(("voice-only-person", entry.channel, user));
		let Some((rect, response)) = tree_row(ui, ROW, id) else {
			return;
		};
		let colors = design::palette(ui);
		let own = state.user.as_ref().is_some_and(|me| me.id == user);
		let hovered = response.hovered() || response.has_focus();
		if own {
			ui.painter().rect_filled(rect, 4, colors.selected);
		} else if hovered {
			paint_hover(ui, rect);
		}
		let shown = crate::voice::shown_participant(state, entry);
		let tint = if self.is_speaking(state, entry.channel, &entry.participant) {
			colors.positive
		} else if shown.muted || shown.server_muted {
			colors.danger
		} else {
			colors.muted
		};
		let row = rect.shrink2(egui::vec2(4.0, 0.0));
		let glyph_left = row.left() + 3.0 * INDENT;
		icons::paint(
			ui.painter(),
			icons::Icon::Speaker,
			glyph(row, glyph_left),
			tint,
		);
		// Deafening implies muted; Discord shows only the headphones then.
		let status = if shown.deafened || shown.server_deafened {
			Some((
				icons::Icon::HeadphonesSlash,
				if shown.server_deafened {
					"voice-voice-participant-deafened-by-server"
				} else {
					"voice-voice-participant-deafened"
				},
			))
		} else if shown.muted || shown.server_muted {
			Some((
				icons::Icon::MicrophoneSlash,
				if shown.server_muted {
					"voice-voice-participant-muted-by-server"
				} else {
					"voice-voice-participant-microphone-muted"
				},
			))
		} else {
			None
		};
		let mut right = row.right();
		if own {
			right = paint_you(ui, row, right) - 4.0;
		}
		// Mute/deafen symbols sit left of the name, beside the speaker glyph, as in Ventrilo.
		let mut left = glyph_left + GLYPH + 4.0;
		if let Some((icon, _)) = status {
			icons::paint(ui.painter(), icon, glyph(row, left), colors.muted);
			left += GLYPH + 4.0;
		}
		let (person, name) = crate::voice::resolve_member(state, entry);
		let font = egui::FontId::new(13.0, design::medium_family(ui.ctx()));
		let area = row.with_min_x(left).with_max_x(right);
		let color = if own || hovered {
			colors.text_strong
		} else {
			colors.text
		};
		paint_text(ui, area, name, font, color);
		response.widget_info(|| {
			let mut label = name.to_owned();
			if let Some((_, key)) = status {
				label.push_str(", ");
				label.push_str(&crate::i18n::translate(key));
			}
			if own {
				label.push_str(", ");
				label.push_str(&crate::i18n::translate("pending-show-you"));
			}
			egui::WidgetInfo::selected(egui::Role::Button, true, own, label)
		});
		if let Some(person) = person {
			self.profile.person_click(ui, &response, None, person);
		}
		if let Some((_, key)) = status {
			response.on_hover_text(crate::i18n::translate(key));
		}
	}
	/// The strip above the tree: the server picker, this account's name, and the call's phase
	/// while one is active. Picking a server loads it like the server rail and hides the call
	/// window; the call itself is untouched.
	fn voice_only_header(
		&mut self,
		ui: &mut egui::Ui,
		state: &mut State,
		commands: &mut Vec<Command>,
	) {
		let current = self.voice_only_current_guild(state);
		let mut strip = header_strip(ui);
		if let Some(call) = &state.voice.active {
			let colors = design::palette(ui);
			let color = match call.phase {
				Phase::Connected | Phase::Waiting => colors.positive,
				Phase::Failed => colors.danger,
				_ => colors.warning,
			};
			let phase = design::medium(&strip, call.phase.label(), 12.0);
			strip.add(egui::Label::new(phase.color(color)).selectable(false));
		}
		let mut picked = None;
		strip.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
			let name = current
				.and_then(|guild| state.guilds.iter().find(|known| known.id == guild))
				.map_or_else(
					|| crate::i18n::translate("switcher-kind-server"),
					|guild| guild.name.clone(),
				);
			let button = ui.add(
				egui::Button::new(
					design::semibold(ui, name.clone(), 14.0).color(design::palette(ui).text_strong),
				)
				.frame(false),
			);
			button.widget_info(|| {
				egui::WidgetInfo::labeled(
					egui::Role::Button,
					true,
					format!(
						"{}, {}",
						name,
						crate::i18n::translate("switcher-kind-server")
					),
				)
			});
			egui::Popup::menu(&button).show(|ui| {
				for guild in &state.guilds {
					if ui
						.selectable_label(current == Some(guild.id), &guild.name)
						.clicked()
					{
						picked = Some(guild.id);
						egui::Popup::close_all(ui.ctx());
					}
				}
			});
			ui.add_space(8.0);
			let account = state.user.as_ref().map_or_else(
				|| crate::i18n::translate("your-account"),
				|user| user.name.clone(),
			);
			let account = design::medium(ui, account, 12.0).color(design::palette(ui).muted);
			ui.add(egui::Label::new(account).truncate().selectable(false));
		});
		if let Some(guild) = picked {
			self.voice_only_guild = Some(guild);
			self.voice_only_stage = None;
			if let Some(command) = state.select_guild(guild) {
				commands.push(command);
			}
		}
	}
}

/// A header strip across `ui` with a rule under it, laid out from the right.
fn header_strip(ui: &mut egui::Ui) -> egui::Ui {
	let (rect, _) = ui.allocate_exact_size(
		egui::vec2(ui.available_width(), HEADER),
		egui::Sense::hover(),
	);
	let rule = egui::Stroke::new(1.0, design::palette(ui).border);
	ui.painter()
		.hline(rect.x_range(), rect.bottom() - 0.5, rule);
	ui.new_child(
		egui::UiBuilder::new()
			.max_rect(rect.shrink2(egui::vec2(10.0, 0.0)))
			.layout(egui::Layout::right_to_left(egui::Align::Center)),
	)
}

/// A strip's title, elided in the room its right-hand items leave.
fn strip_title(strip: &mut egui::Ui, title: String) {
	strip.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
		let title = design::semibold(ui, title, 14.0).color(design::palette(ui).text_strong);
		ui.add(egui::Label::new(title).truncate().selectable(false));
	});
}

/// A server's header row; a click folds its channels away. Returns whether they are shown.
fn guild_row(ui: &mut egui::Ui, guild: &model::Guild) -> bool {
	let id = egui::Id::unique(("voice-only-server", guild.id));
	let mut fold = CollapsingState::load_with_default_open(ui.ctx(), id, true);
	let Some((rect, response)) = tree_row(ui, SERVER_ROW, id.with("row")) else {
		return fold.is_open();
	};
	if response.clicked() {
		fold.toggle(ui);
		fold.store(ui.ctx());
	}
	let open = fold.is_open();
	let colors = design::palette(ui);
	let hovered = response.hovered() || response.has_focus();
	if hovered {
		paint_hover(ui, rect);
	}
	let color = if hovered {
		colors.text_strong
	} else {
		colors.text
	};
	let row = rect.shrink2(egui::vec2(4.0, 0.0));
	paint_caret(ui, row, row.left(), open, color);
	let font = egui::FontId::new(13.0, design::semibold_family(ui.ctx()));
	let name = row.with_min_x(row.left() + INDENT + 2.0);
	paint_text(ui, name, &guild.name, font, color);
	response.widget_info(|| {
		egui::WidgetInfo::labeled(
			egui::Role::Button,
			true,
			format!(
				"{}, {}",
				guild.name,
				crate::i18n::translate(if open {
					"categories-category-header-expanded"
				} else {
					"categories-category-header-collapsed"
				})
			),
		)
	});
	response.on_hover_text(crate::i18n::translate(if open {
		"categories-category-header-collapse"
	} else {
		"categories-category-header-expand"
	}));
	open
}

/// Allocates a full-width tree row. Rows scrolled out of view only take their space, so a long
/// tree lays out and registers just the visible ones.
fn tree_row(ui: &mut egui::Ui, height: f32, id: egui::Id) -> Option<(egui::Rect, egui::Response)> {
	let (_, rect) = ui.allocate_space(egui::vec2(ui.available_width(), height));
	ui.is_rect_visible(rect)
		.then(|| (rect, ui.interact(rect, id, egui::Sense::click())))
}

/// The channel list's order among voice channels: uncategorized first, then by category.
fn list_order(state: &State, channel: &model::Channel) -> (Option<(i32, Id)>, i32, Id) {
	let category = channel
		.parent_id
		.and_then(|id| state.channel(id))
		.filter(|parent| parent.kind == 4 && parent.guild == channel.guild)
		.map(|parent| (parent.position, parent.id));
	(category, channel.position, channel.id)
}

/// Hover fill of a tree row; translucent over a window image, like the channel list's rows.
fn paint_hover(ui: &egui::Ui, rect: egui::Rect) {
	let fill = design::row_highlight(ui, design::palette(ui).hover, 1.0);
	ui.painter().rect_filled(rect, 4, fill);
}

/// The fold caret in the indent step starting at `left`.
fn paint_caret(ui: &egui::Ui, row: egui::Rect, left: f32, open: bool, color: egui::Color32) {
	let icon = if open {
		icons::Icon::ChevronDown
	} else {
		icons::Icon::ChevronRight
	};
	let center = egui::pos2(left + INDENT * 0.5, row.center().y);
	icons::paint(
		ui.painter(),
		icon,
		egui::Rect::from_center_size(center, egui::Vec2::splat(12.0)),
		color,
	);
}

/// The glyph cell starting at `left`, centred on `row`.
fn glyph(row: egui::Rect, left: f32) -> egui::Rect {
	let center = egui::pos2(left + GLYPH * 0.5, row.center().y);
	egui::Rect::from_center_size(center, egui::Vec2::splat(GLYPH))
}

/// One line of `text` centred in `area`, elided when it does not fit.
fn paint_text(
	ui: &egui::Ui,
	area: egui::Rect,
	text: &str,
	font: egui::FontId,
	color: egui::Color32,
) {
	let mut job = egui::text::LayoutJob::simple_singleline(text.to_owned(), font, color);
	job.wrap = egui::text::TextWrapping::truncate_at_width(area.width().max(1.0));
	let galley = ui.painter().layout_job(job);
	let pos = egui::pos2(area.left(), area.center().y - galley.size().y * 0.5);
	ui.painter().galley(pos, galley, color);
}

/// The own row's "You" tag, ending at `right`; returns where it starts.
fn paint_you(ui: &egui::Ui, row: egui::Rect, right: f32) -> f32 {
	let colors = design::palette(ui);
	let font = egui::FontId::new(10.0, design::semibold_family(ui.ctx()));
	let you = crate::i18n::translate("pending-show-you");
	let galley = ui.painter().layout_no_wrap(you, font, colors.accent_text);
	let tag = egui::Rect::from_min_max(
		egui::pos2(right - galley.size().x - 8.0, row.center().y - 7.0),
		egui::pos2(right, row.center().y + 7.0),
	);
	ui.painter().rect_filled(tag, 3, colors.accent);
	let text = tag.center() - galley.size() * 0.5;
	ui.painter().galley(text, galley, colors.accent_text);
	tag.left()
}

#[cfg(test)]
mod tests {
	use crate::MessagingUi;
	use client_core::{
		Envelope, Event, State,
		voice::{Participant, RosterEntry},
	};
	use model::{Channel, Guild, Id, Member, User};

	fn user(id: u64, name: &str) -> User {
		User {
			kind: Default::default(),
			webhook: false,
			id: Id(id),
			name: name.into(),
			avatar: None,
			discriminator: 0,
			primary_guild: None,
			cosmetics: None,
		}
	}

	fn channel(id: u64, kind: u8, name: &str) -> Channel {
		Channel {
			icon: None,
			last_message: None,
			id: Id(id),
			guild: Some(Id(10)),
			parent_id: None,
			position: 0,
			name: name.into(),
			kind,
			recipients: vec![],
			member_list_id: None,
			message_count: None,
			user_limit: None,
			slowmode: 0,
			tags: None,
		}
	}

	/// One server with an open text channel and a voice channel that one person is in.
	fn one_server() -> State {
		let mut state = State {
			demo: true,
			..State::default()
		};
		state.apply(Envelope {
			generation: state.generation,
			event: Event::Ready {
				permissions: Default::default(),
				user: user(1, "Voice tester"),
				guilds: vec![Guild {
					default_message_notifications: None,
					stickers: None,
					emojis: None,
					id: Id(10),
					name: "Voice club".into(),
					icon: None,
					banner: None,
				}],
				channels: vec![channel(20, 0, "plans-and-notes"), channel(30, 2, "Lobby")],
			},
		});
		let permissions = test_support::permission_snapshot(&state);
		state.permissions.replace(permissions).unwrap();
		state.voice.roster = vec![RosterEntry {
			guild: Id(10),
			channel: Id(30),
			participant: Participant {
				user: Id(7),
				muted: true,
				deafened: false,
				server_muted: false,
				server_deafened: false,
				video: false,
				streaming: false,
			},
			member: Some(Member {
				roles: vec![],
				user: user(7, "Quill Harper"),
				nick: None,
				status: None,
				custom_status: None,
				activities: vec![],
				clients: Default::default(),
			}),
		}];
		let _ = state.select(Id(20));
		state
	}

	/// The client in its own context, drawn frame by frame on a steady clock.
	struct Client {
		ctx: egui::Context,
		view: MessagingUi,
		time: f64,
	}

	impl Client {
		fn new(voice_only: bool) -> Self {
			let ctx = egui::Context::default();
			ctx.enable_accesskit();
			let mut view = MessagingUi::default();
			if voice_only {
				view.set_interface_tweaks([extensions::InterfaceTweak::VoiceOnlyLayout].into());
			}
			Self {
				ctx,
				view,
				time: 0.0,
			}
		}

		/// One frame with `events`: each painted text with a point inside it, and the accessible
		/// names.
		fn frame(
			&mut self,
			state: &mut State,
			events: Vec<egui::Event>,
		) -> (Vec<(String, egui::Pos2)>, Vec<String>) {
			self.time += 0.05;
			let view = &mut self.view;
			let output = self.ctx.run_ui(
				egui::RawInput {
					screen_rect: Some(egui::Rect::from_min_size(
						egui::Pos2::ZERO,
						egui::vec2(1200.0, 800.0),
					)),
					time: Some(self.time),
					events,
					..Default::default()
				},
				|ui| {
					view.show(ui, state);
				},
			);
			let mut texts = Vec::new();
			let mut stack: Vec<_> = output.shapes.iter().map(|clipped| &clipped.shape).collect();
			while let Some(shape) = stack.pop() {
				match shape {
					egui::Shape::Text(text) => {
						let point = text.pos + egui::vec2(4.0, 4.0);
						texts.push((text.galley.text().to_owned(), point));
					}
					egui::Shape::Vec(shapes) => stack.extend(shapes),
					_ => {}
				}
			}
			let names = output
				.platform_output
				.accesskit_update
				.iter()
				.flat_map(|update| update.nodes.iter())
				.filter_map(|(_, node)| node.label().map(str::to_owned))
				.collect();
			output.drop_without_applying_deltas();
			(texts, names)
		}
	}

	/// Checks that the voice-only layout lists the voice channel and the person in it, keeps the
	/// settings button, and draws neither the text channel nor its message box.
	#[test]
	fn voice_only_layout_shows_voice_tree_instead_of_chat() {
		let mut state = one_server();
		let mut client = Client::new(true);
		client.frame(&mut state, vec![]);
		let (texts, names) = client.frame(&mut state, vec![]);
		let shown = |needle: &str| texts.iter().any(|(text, _)| text.contains(needle));
		assert!(shown("Lobby"), "voice channel listed: {texts:?}");
		assert!(shown("Quill Harper"), "participant listed: {texts:?}");
		assert!(!shown("plans-and-notes"), "text channel hidden: {texts:?}");
		let placeholder = crate::i18n::translate_args(
			"composer-placeholder-channel",
			&[("channel", "plans-and-notes")],
		);
		assert!(!shown(&placeholder), "message box hidden: {texts:?}");
		let settings = crate::i18n::translate("user-settings");
		assert!(names.contains(&settings), "settings reachable: {names:?}");
	}

	/// Checks that without the tweak the usual layout still lists the text channel.
	#[test]
	fn usual_layout_keeps_text_channels() {
		let mut state = one_server();
		let mut client = Client::new(false);
		client.frame(&mut state, vec![]);
		let (texts, _) = client.frame(&mut state, vec![]);
		assert!(
			texts
				.iter()
				.any(|(text, _)| text.contains("plans-and-notes")),
			"{texts:?}"
		);
	}

	/// Checks the voice fixture under the voice-only layout: its call window stays hidden during
	/// the call, a click only selects, a double click or Enter on the focused row opens it, a
	/// second double click or Enter closes it, and leaving hides it again.
	#[test]
	fn voice_only_call_window_opens_on_double_click() {
		let mut state = test_support::voice_demo_state();
		let room = state.voice.active.as_ref().expect("fixture call").channel;
		let hide = crate::i18n::translate("ventrilo-hide-call");
		let mut client = Client::new(true);
		client.frame(&mut state, vec![]);
		let (texts, names) = client.frame(&mut state, vec![]);
		assert!(
			client.view.voice_only_stage.is_none(),
			"hidden during the call"
		);
		assert!(!names.contains(&hide), "no call window: {names:?}");
		let row = texts
			.iter()
			.find(|(text, _)| text == "Room 3,5")
			.map(|(_, point)| *point)
			.expect("voice channel row");
		let click = |client: &mut Client, state: &mut State| {
			for pressed in [true, false] {
				let event = egui::Event::PointerButton {
					pos: row,
					button: egui::PointerButton::Primary,
					pressed,
					modifiers: egui::Modifiers::NONE,
				};
				client.frame(state, vec![event]);
			}
		};
		client.frame(&mut state, vec![egui::Event::PointerMoved(row)]);
		click(&mut client, &mut state);
		assert!(
			client.view.voice_only_stage.is_none(),
			"a click only selects"
		);
		click(&mut client, &mut state);
		let (_, names) = client.frame(&mut state, vec![]);
		assert_eq!(
			client.view.voice_only_stage.map(|stage| stage.channel),
			Some(room)
		);
		assert!(names.contains(&hide), "call window open: {names:?}");
		// A second double click closes the window again; the call itself stays.
		for _ in 0..8 {
			client.frame(&mut state, vec![]);
		}
		click(&mut client, &mut state);
		click(&mut client, &mut state);
		let (_, names) = client.frame(&mut state, vec![]);
		assert!(
			client.view.voice_only_stage.is_none(),
			"double click again closes"
		);
		assert!(!names.contains(&hide), "closed again: {names:?}");
		assert!(state.voice.active.is_some(), "closing keeps the call");
		// Keyboard activation toggles the focused row the same way.
		let row_id = egui::Id::unique(("voice-only-channel", room)).with("row");
		let enter = || egui::Event::Key {
			key: egui::Key::Enter,
			physical_key: None,
			pressed: true,
			repeat: false,
			modifiers: egui::Modifiers::NONE,
		};
		client.ctx.memory_mut(|memory| memory.request_focus(row_id));
		client.frame(&mut state, vec![]);
		client.frame(&mut state, vec![enter()]);
		assert_eq!(
			client.view.voice_only_stage.map(|stage| stage.channel),
			Some(room),
			"Enter opens the focused channel"
		);
		client.frame(&mut state, vec![enter()]);
		assert!(
			client.view.voice_only_stage.is_none(),
			"Enter again closes it"
		);
		client.frame(&mut state, vec![enter()]);
		assert_eq!(
			client.view.voice_only_stage.map(|stage| stage.channel),
			Some(room),
			"Enter reopens it"
		);
		client.frame(&mut state, vec![]);
		state.voice.active = None;
		let (_, names) = client.frame(&mut state, vec![]);
		assert!(client.view.voice_only_stage.is_none(), "leaving hides it");
		assert!(!names.contains(&hide), "call window closed: {names:?}");
	}

	fn channel_in(id: u64, kind: u8, name: &str, guild: u64) -> Channel {
		Channel {
			guild: Some(Id(guild)),
			..channel(id, kind, name)
		}
	}

	/// A second server whose permissions never loaded, plus an explicitly denied channel in
	/// the first server.
	fn two_servers() -> State {
		let mut state = one_server();
		state.channels.push(channel_in(32, 2, "Staff only", 10));
		let mut snapshot = test_support::permission_snapshot(&state);
		for channel in &mut snapshot.channels {
			if channel.id == Id(32) {
				channel.overwrites = Some(vec![model::permissions::Overwrite {
					id: Id(10),
					kind: 0,
					allow: 0,
					deny: model::permissions::VIEW_CHANNEL,
				}]);
			}
		}
		state.permissions.replace(snapshot).unwrap();
		state.guilds.push(Guild {
			default_message_notifications: None,
			stickers: None,
			emojis: None,
			id: Id(11),
			name: "Other club".into(),
			icon: None,
			banner: None,
		});
		state.channels.push(channel_in(31, 2, "Backroom", 11));
		state
	}

	/// The header picker lists every server; picking one loads its voice channels even though
	/// its permissions never loaded, while an explicitly denied channel stays hidden.
	#[test]
	fn voice_only_server_picker_switches_servers() {
		let mut state = two_servers();
		let mut client = Client::new(true);
		client.frame(&mut state, vec![]);
		let (texts, _) = client.frame(&mut state, vec![]);
		let shown = |texts: &[(String, egui::Pos2)], needle: &str| {
			texts.iter().any(|(text, _)| text.contains(needle))
		};
		assert!(shown(&texts, "Lobby"), "open server's channel: {texts:?}");
		assert!(
			!shown(&texts, "Backroom"),
			"other server waits for the pick: {texts:?}"
		);
		assert!(
			!shown(&texts, "Staff only"),
			"denied channel hidden: {texts:?}"
		);
		let click = |client: &mut Client, state: &mut State, pos: egui::Pos2| {
			for pressed in [true, false] {
				client.frame(
					state,
					vec![egui::Event::PointerButton {
						pos,
						button: egui::PointerButton::Primary,
						pressed,
						modifiers: egui::Modifiers::NONE,
					}],
				);
			}
		};
		// Shape order groups layers, so target the header band (below the title bar, above
		// the tree) instead of the first painted server name.
		let picker = texts
			.iter()
			.find(|(text, point)| text == "Voice club" && point.y > 32.0 && point.y < 60.0)
			.map(|(_, point)| *point)
			.expect("server picker");
		click(&mut client, &mut state, picker);
		let (texts, _) = client.frame(&mut state, vec![]);
		let other = texts
			.iter()
			.find(|(text, _)| text == "Other club")
			.map(|(_, point)| *point)
			.expect("picker lists the unloaded server");
		click(&mut client, &mut state, other);
		let (texts, _) = client.frame(&mut state, vec![]);
		assert!(
			shown(&texts, "Backroom"),
			"picked server lists despite unknown permissions: {texts:?}"
		);
		assert!(
			!shown(&texts, "Lobby"),
			"previous server's channels gone: {texts:?}"
		);
	}
}
