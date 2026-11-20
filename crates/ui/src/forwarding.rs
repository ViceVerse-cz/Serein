use crate::{avatars::Avatars, design, dialog, switcher};
use client_core::{Command, State};
use model::{Delivery, Id};

/// Destinations listed at once, like the official client; searching finds the rest.
const DESTINATIONS: usize = 16;

/// The most recently active writable conversations matching `query`.
fn destinations(state: &State, query: &str) -> Vec<Id> {
	let query = switcher::bounded(query).to_lowercase();
	let words: Vec<_> = query.split_whitespace().collect();
	let mut label = String::new();
	let mut matched = vec![false; words.len()];
	let mut found: Vec<_> = state
		.channels
		.iter()
		.filter(|c| {
			state.can_compose(c.id)
				&& switcher::channel_matches(state, c, &words, &mut label, &mut matched)
		})
		.collect();
	switcher::recent_first(&mut found, DESTINATIONS, |c| switcher::activity(c));
	found.into_iter().map(|c| c.id).collect()
}

#[derive(Default)]
pub(super) struct ForwardDialog {
	source: Option<(u64, Id, Id)>,
	query: String,
	note: String,
	targets: Vec<Id>,
	/// Results for `searched`, rebuilt when the query changes and at most once a second otherwise.
	results: Vec<Id>,
	searched: Option<(String, f64)>,
	/// At most ten nonce strings: one forward and optional note per destination.
	sent: Vec<(Id, String)>,
	focus: bool,
	error: Option<&'static str>,
}
impl ForwardDialog {
	pub fn open(&mut self, state: &State, message: Id) {
		if state.can_forward(message)
			&& let Some(channel) = state.selected
		{
			*self = Self {
				source: Some((state.generation, channel, message)),
				focus: true,
				..Self::default()
			};
		}
	}
	pub fn show(
		&mut self,
		ctx: &egui::Context,
		state: &mut State,
		avatars: &mut Avatars,
		commands: &mut Vec<Command>,
	) {
		let Some((generation, channel, message)) = self.source else {
			return;
		};
		if generation != state.generation
			|| state.selected != Some(channel)
			|| !state.can_read_history(channel)
		{
			*self = Self::default();
			return;
		}
		let mut submit = false;
		let mut close = false;
		let submitted = !self.sent.is_empty();
		let response = dialog::Dialog::new(
			"forward-message",
			crate::i18n::translate("forwarding-show-forward-to"),
		)
		.subtitle(crate::i18n::translate(
			"forwarding-show-select-where-you-want-to-share-this-message",
		))
		.width(480.0)
		.show(ctx, |d| {
			d.content(|ui| {
				let input = design::input(
					ui,
					egui::TextEdit::singleline(&mut self.query)
						.align(egui::Align2::LEFT_CENTER)
						.hint_text(crate::i18n::translate("forwarding-show-search"))
						.char_limit(256)
						.desired_width(f32::INFINITY),
				);
				if std::mem::take(&mut self.focus) {
					input.request_focus();
				}
				design::hint(
					ui,
					&format!(
						"{} / 5 {}",
						self.targets.len(),
						crate::i18n::translate("forwarding-show-destinations-selected")
					),
				);
				ui.add_space(8.0);
			});
			d.content(|ui| {
				egui::ScrollArea::vertical()
					.max_height((ctx.content_rect().height() - 340.0).clamp(80.0, 280.0))
					.auto_shrink([false, true])
					.show(ui, |ui| {
						let colors = design::palette(ui);
						let now = ui.input(|input| input.time);
						if self.searched.as_ref().is_none_or(|(query, at)| {
							*query != self.query || !(0.0..1.0).contains(&(now - at))
						}) {
							self.results = destinations(state, &self.query);
							self.searched = Some((self.query.clone(), now));
						}
						// Chosen destinations stay listed so they can be removed after searching.
						let rows = self
							.targets
							.iter()
							.filter(|id| !self.results.contains(id))
							.chain(&self.results)
							.copied()
							.collect::<Vec<_>>();
						let mut found = false;
						for target in rows.into_iter().filter_map(|id| state.channel(id)) {
							let guild = target.guild.and_then(|id| state.guild(id));
							let context = guild.map_or_else(
								|| crate::i18n::translate("forwarding-show-direct-messages"),
								|g| g.name.clone(),
							);
							found = true;
							let selected = self.targets.contains(&target.id);
							let enabled = !submitted
								&& (selected || self.targets.len() < 5)
								&& state.can_compose(target.id);
							let response = ui
								.push_id(target.id.0, |ui| {
									ui.add_enabled_ui(enabled, |ui| {
										design::selection_row(
											ui,
											selected,
											&target.name,
											&context,
											|ui, rect| {
												if let Some(guild) = guild {
													avatars.paint_guild(
														ui, guild, rect, state.demo, 8,
													);
													let badge = egui::Rect::from_center_size(
														rect.right_bottom() - egui::vec2(2.0, 2.0),
														egui::Vec2::splat(16.0),
													);
													ui.painter().circle_filled(
														badge.center(),
														9.0,
														colors.raised,
													);
													crate::icons::paint(
														ui.painter(),
														crate::icons::Icon::Hash,
														badge.shrink(2.0),
														colors.muted,
													);
												} else if let Some(user) = target
													.recipients
													.first()
													.filter(|_| target.kind == 1)
												{
													avatars.paint_user(
														ui, user, 32.0, rect, state.demo,
													);
												} else {
													design::paint_avatar(
														ui,
														&target.name,
														32.0,
														rect,
													);
												}
											},
										)
									})
									.inner
								})
								.inner;
							if response.clicked() {
								if selected {
									self.targets.retain(|id| *id != target.id);
								} else {
									self.targets.push(target.id);
								}
							}
						}
						if !found {
							ui.label(crate::i18n::translate(
								"forwarding-show-no-matching-destinations",
							));
						}
					});
			});
			d.content(|ui| {
				crate::design::card_divider(ui);
				ui.add_space(2.0);
				if let Some(source) = state.timeline.get(message) {
					let preview: String = if source.content.contains("||") {
						"[Spoiler hidden]".into()
					} else {
						source
							.content
							.chars()
							.take(240)
							.map(|c| if c.is_whitespace() { ' ' } else { c })
							.collect()
					};
					if preview.is_empty() {
						ui.label(crate::i18n::translate(
							"forwarding-show-attachment-or-embedded-content",
						));
					} else {
						ui.label(&preview);
					}
				} else {
					ui.label(crate::i18n::translate(
						"forwarding-show-source-message-is-no-longer-available",
					));
				}
				ui.add_space(8.0);
				ui.add_enabled_ui(!submitted, |ui| {
					design::input(
						ui,
						egui::TextEdit::singleline(&mut self.note)
							.align(egui::Align2::LEFT_CENTER)
							.hint_text(crate::i18n::translate(
								"forwarding-show-add-an-optional-message",
							))
							.char_limit(
								client_core::MAX_CONTENT + model::message_options::PREFIX_ALLOWANCE,
							)
							.desired_width(f32::INFINITY),
					)
				});
				if let Some(error) = self.error {
					design::notice(ui, design::Level::Error, error);
				}
				if submitted {
					for target in &self.targets {
						let pending: Vec<_> = self
							.sent
							.iter()
							.filter(|(id, _)| id == target)
							.filter_map(|(_, nonce)| {
								state.pending.iter().find(|p| &p.nonce == nonce)
							})
							.collect();
						let status = crate::i18n::translate_if_key(
							if pending.iter().any(|p| p.delivery == Delivery::Ambiguous) {
								"forwarding-show-outcome-unknown-check-the-destination-before-resending"
							} else if pending.iter().any(|p| p.delivery == Delivery::Rejected) {
								"forwarding-show-a-message-failed-check-the-destination"
							} else if pending.is_empty() {
								"forwarding-show-sent"
							} else {
								"forwarding-show-sending"
							},
						);
						let conversation = state.channel(*target).map_or_else(
							|| crate::i18n::translate("forwarding-show-conversation"),
							|c| c.name.clone(),
						);
						ui.label(format!("{conversation}: {status}"));
					}
				}
			});
			d.footer(|ui| {
				if !submitted {
					submit = ui
						.add_enabled_ui(
							!self.targets.is_empty() && state.can_forward(message),
							|ui| {
								dialog::action(ui, "forwarding-show-send", dialog::Action::Primary)
							},
						)
						.inner
						.clicked();
				}
				close = dialog::action(
					ui,
					if submitted {
						"forwarding-show-done"
					} else {
						"forwarding-show-cancel"
					},
					dialog::Action::Neutral,
				)
				.clicked();
			});
		});
		if submit {
			let outgoing = state.prepare_forward(message, &self.targets, &self.note);
			self.error = outgoing.is_empty().then_some(state.status);
			for command in &outgoing {
				if let Command::Forward { channel, nonce, .. }
				| Command::Send { channel, nonce, .. } = command
				{
					self.sent.push((*channel, nonce.clone()));
				}
			}
			commands.extend(outgoing);
		}
		if close || response.close {
			*self = Self::default();
		}
	}
}

/// Discord's source line under a forward: the server (or direct message) it came from and when
/// it was first sent. Only a joined server or a loaded conversation is named, and only then does
/// the line open that message; returns its link when clicked. The link is kept out of the
/// text-selection `surface` so a click opens it instead of starting a selection.
pub(super) fn origin(
	ui: &mut egui::Ui,
	state: &State,
	key: Id,
	origin: &model::ForwardOrigin,
	images: &mut Avatars,
	demo: bool,
	surface: &mut crate::select::Surface,
) -> Option<String> {
	let colors = design::palette(ui);
	let place = match origin.guild {
		Some(guild) => state
			.guild(guild)
			.map(|guild| (Some(guild), None, guild.name.as_str())),
		None => state
			.channel(origin.channel)
			.filter(|channel| channel.guild.is_none() && matches!(channel.kind, 1 | 3))
			.map(|channel| {
				let icon = crate::channel_pill::icon(channel, &state.channels);
				(None, Some(icon), state.conversation_name(channel))
			}),
	};
	// Fixture IDs do not encode a real creation timestamp.
	let time = (origin.message.0 >= 1 << 22)
		.then(|| {
			time::OffsetDateTime::from_unix_timestamp(
				model::threads::snowflake_ms(origin.message) / 1000,
			)
			.ok()
		})
		.flatten()
		.map(crate::local_time::calendar);
	if place.is_none() && time.is_none() {
		return None;
	}
	let id = ui.scope_id().with(("forward-origin", key));
	let hovered = place.is_some()
		&& ui
			.ctx()
			.read_response(id)
			.is_some_and(|response| response.hovered());
	let color = if hovered { colors.text } else { colors.muted };
	let text = |text: &str| {
		egui::Label::new(egui::RichText::new(text).size(13.0).color(color)).selectable(false)
	};
	let row = ui.horizontal(|ui| {
		let gap = 4.0;
		ui.spacing_mut().item_spacing.x = gap;
		if let Some((guild, icon, name)) = place {
			let (rect, _) = ui.allocate_exact_size(egui::Vec2::splat(16.0), egui::Sense::hover());
			match (guild, icon) {
				(Some(guild), _) => images.paint_guild(ui, guild, rect, demo, 4),
				(None, Some(icon)) => crate::icons::paint(ui.painter(), icon, rect, color),
				(None, None) => {}
			}
			// A long name truncates before the time and chevron, never pushing them out.
			let font = egui::FontId::proportional(13.0);
			let width = |text: &str| {
				ui.painter()
					.layout_no_wrap(text.to_owned(), font.clone(), color)
					.size()
					.x
			};
			let trailing = time
				.as_deref()
				.map_or(0.0, |time| width("•") + width(time) + 2.0 * gap)
				+ 12.0 + gap;
			ui.scope(|ui| {
				ui.set_max_width((ui.available_width() - trailing).max(0.0));
				ui.add(text(name).truncate());
			});
			if time.is_some() {
				ui.add(text("•"));
			}
		}
		if let Some(time) = &time {
			ui.add(text(time));
		}
		if place.is_some() {
			let (rect, _) = ui.allocate_exact_size(egui::Vec2::splat(12.0), egui::Sense::hover());
			crate::icons::paint(ui.painter(), crate::icons::Icon::ChevronRight, rect, color);
		}
	});
	let (_, _, name) = place?;
	let response = ui.interact(row.response.rect, id, egui::Sense::click());
	surface.keep(&response);
	let label = time.map_or_else(|| name.to_owned(), |time| format!("{name} • {time}"));
	response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Link, ui.is_enabled(), &label));
	let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
	response.clicked().then(|| {
		let scope = origin
			.guild
			.map_or_else(|| "@me".into(), |guild| guild.to_string());
		format!(
			"https://discord.com/channels/{scope}/{}/{}",
			origin.channel, origin.message
		)
	})
}
