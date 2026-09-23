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
		let response = dialog::Dialog::new("forward-message", "Forward To")
			.subtitle("Select where you want to share this message.")
			.width(480.0)
			.show(ctx, |d| {
				d.content(|ui| {
					let input = design::input(
						ui,
						egui::TextEdit::singleline(&mut self.query)
							.hint_text("Search")
							.char_limit(256)
							.desired_width(f32::INFINITY),
					);
					if std::mem::take(&mut self.focus) {
						input.request_focus();
					}
					design::hint(
						ui,
						&format!("{} of 5 destinations selected", self.targets.len()),
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
								let context = guild.map_or("Direct Messages", |g| g.name.as_str());
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
												context,
												|ui, rect| {
													if let Some(guild) = guild {
														avatars.paint_guild(
															ui, guild, rect, state.demo, 8,
														);
														let badge = egui::Rect::from_center_size(
															rect.right_bottom()
																- egui::vec2(2.0, 2.0),
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
								ui.label("No matching destinations");
							}
						});
				});
				d.content(|ui| {
					ui.separator();
					ui.add_space(8.0);
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
						ui.label(if preview.is_empty() {
							"Attachment or embedded content"
						} else {
							&preview
						});
					} else {
						ui.label("Source message is no longer available");
					}
					ui.add_space(8.0);
					ui.add_enabled_ui(!submitted, |ui| {
						design::input(
							ui,
							egui::TextEdit::singleline(&mut self.note)
								.hint_text("Add an optional message…")
								.char_limit(client_core::MAX_CONTENT)
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
							let status =
								if pending.iter().any(|p| p.delivery == Delivery::Ambiguous) {
									"Outcome unknown — check the destination before resending"
								} else if pending.iter().any(|p| p.delivery == Delivery::Rejected) {
									"A message failed — check the destination"
								} else if pending.is_empty() {
									"Sent"
								} else {
									"Sending…"
								};
							ui.label(format!(
								"{}: {status}",
								state
									.channel(*target)
									.map_or("Conversation", |c| c.name.as_str())
							));
						}
					}
				});
				d.footer(|ui| {
					if !submitted {
						submit = ui
							.add_enabled_ui(
								!self.targets.is_empty() && state.can_forward(message),
								|ui| dialog::action(ui, "Send", dialog::Action::Primary),
							)
							.inner
							.clicked();
					}
					close = dialog::action(
						ui,
						if submitted { "Done" } else { "Cancel" },
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
