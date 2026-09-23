//! Session-bound server creation and explicit, user-confirmed invite joining.
use crate::{design, dialog, icons, invites::input_code};
use client_core::{Command, State};
use model::Id;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Page {
	#[default]
	Join,
	Choose,
	Audience,
	Customize,
}

fn invite_input(ui: &mut egui::Ui, text: &mut String, focus: bool) -> egui::Response {
	let colors = design::palette(ui);
	let response = ui.add_sized(
		[ui.available_width(), 48.0],
		egui::TextEdit::singleline(text)
			.hint_text("https://discord.gg/hTKzmak")
			.font(egui::FontId::proportional(16.0))
			.align(egui::Align2::LEFT_CENTER)
			.frame(
				egui::Frame::new()
					.fill(colors.base)
					.corner_radius(8)
					.inner_margin(egui::Margin::symmetric(12, 8)),
			)
			.char_limit(512),
	);
	if focus {
		response.request_focus();
	}
	let stroke = if response.has_focus() {
		egui::Stroke::new(2.0, colors.accent)
	} else {
		egui::Stroke::new(1.0, colors.border)
	};
	ui.painter()
		.rect_stroke(response.rect, 8, stroke, egui::StrokeKind::Inside);
	response
}

fn option_row(ui: &mut egui::Ui, icon: icons::Icon, label: &str) -> egui::Response {
	let colors = design::palette(ui);
	let (rect, response) =
		ui.allocate_exact_size(egui::vec2(ui.available_width(), 56.0), egui::Sense::click());
	response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), label));
	let hot = response.hovered() || response.has_focus();
	ui.painter().rect(
		rect,
		9,
		if hot { colors.hover } else { colors.base },
		egui::Stroke::new(1.0, if hot { colors.accent } else { colors.border }),
		egui::StrokeKind::Inside,
	);
	icons::paint(
		ui.painter(),
		icon,
		egui::Rect::from_center_size(
			egui::pos2(rect.left() + 27.0, rect.center().y),
			egui::Vec2::splat(22.0),
		),
		if hot {
			colors.text_strong
		} else {
			colors.muted
		},
	);
	ui.painter().text(
		egui::pos2(rect.left() + 50.0, rect.center().y),
		egui::Align2::LEFT_CENTER,
		label,
		egui::FontId::new(15.0, design::medium_family(ui.ctx())),
		colors.text,
	);
	icons::paint(
		ui.painter(),
		icons::Icon::ChevronRight,
		egui::Rect::from_center_size(
			egui::pos2(rect.right() - 20.0, rect.center().y),
			egui::Vec2::splat(16.0),
		),
		colors.muted,
	);
	response
}

pub(super) struct JoinDialog {
	generation: Option<u64>,
	page: Page,
	picker: bool,
	input: String,
	focus: bool,
	status: &'static str,
	name: String,
	name_focus: bool,
	icon: Option<String>,
	icon_preview: Option<egui::TextureHandle>,
	icon_request: u64,
	icon_requested: bool,
	icon_pending: bool,
	icon_error: Option<&'static str>,
	submitted: bool,
}

impl Default for JoinDialog {
	fn default() -> Self {
		Self {
			generation: None,
			page: Page::Join,
			picker: false,
			input: String::new(),
			focus: false,
			status: "",
			name: String::new(),
			name_focus: false,
			icon: None,
			icon_preview: None,
			icon_request: 0,
			icon_requested: false,
			icon_pending: false,
			icon_error: None,
			submitted: false,
		}
	}
}

impl JoinDialog {
	pub fn open_picker(&mut self, generation: u64) {
		*self = Self {
			generation: Some(generation),
			page: Page::Choose,
			picker: true,
			..Self::default()
		};
	}
	#[cfg_attr(not(any(test, feature = "demo")), allow(dead_code))]
	pub fn open(&mut self, generation: u64) {
		self.open_with(generation, String::new());
	}
	/// Prefills the field only. The invite is still looked up and joined on confirmation.
	pub fn open_with(&mut self, generation: u64, input: String) {
		*self = Self {
			generation: Some(generation),
			page: Page::Join,
			focus: true,
			input,
			..Self::default()
		};
	}
	pub fn take_icon_request(&mut self) -> Option<(u64, Id, u64)> {
		if !std::mem::take(&mut self.icon_requested) {
			return None;
		}
		self.icon_request = self.icon_request.wrapping_add(1);
		Some((self.generation?, Id(0), self.icon_request))
	}
	pub fn accept_icon(
		&mut self,
		ctx: &egui::Context,
		request: (u64, Id, u64),
		result: Result<Option<(String, egui::ColorImage)>, &'static str>,
	) {
		if self.generation != Some(request.0)
			|| request.1 != Id(0)
			|| request.2 != self.icon_request
			|| !self.icon_pending
		{
			return;
		}
		self.icon_pending = false;
		match result {
			Ok(Some((data, image))) if data.len() <= model::server_settings::MAX_ICON_DATA_URI => {
				self.icon = Some(data);
				self.icon_preview = Some(ctx.load_texture(
					"create-server-icon",
					image,
					egui::TextureOptions::LINEAR,
				));
				self.icon_error = None;
			}
			Ok(Some(_)) => self.icon_error = Some("Prepared icon is too large"),
			Ok(None) => {}
			Err(error) => self.icon_error = Some(error),
		}
	}
	pub fn show(
		&mut self,
		ctx: &egui::Context,
		state: &mut State,
		avatars: &mut crate::avatars::Avatars,
		commands: &mut Vec<Command>,
	) {
		if self.page == Page::Join {
			self.show_join(ctx, state, avatars, commands);
		} else {
			self.show_picker(ctx, state, commands);
		}
	}
	fn show_join(
		&mut self,
		ctx: &egui::Context,
		state: &mut State,
		avatars: &mut crate::avatars::Avatars,
		commands: &mut Vec<Command>,
	) {
		if self.generation != Some(state.generation) {
			*self = Self::default();
			return;
		}
		let mut close = false;
		let response = dialog::Dialog::new("join-server-dialog", "Join a Server")
			.subtitle("Enter an invite below to join an existing server.")
			.width(460.0)
			.show(ctx, |d| {
				let (ready, member, loading, accepted, parsed) =
					d.scroll(260.0, |ui| self.body(ui, state, avatars));
				d.footer(|ui| {
					let busy = loading || state.invite_join.pending;
					let enabled = !state.demo && !busy && !member && !accepted;
					let text = if busy {
						"Please wait…"
					} else if ready {
						"Join Server"
					} else {
						"Check Invite"
					};
					ui.add_enabled_ui(enabled, |ui| {
						if dialog::action(ui, text, dialog::Action::Primary).clicked() {
							self.submit(state, parsed.clone(), ready, commands);
						}
					});
					if dialog::action(
						ui,
						if self.picker { "Back" } else { "Cancel" },
						dialog::Action::Neutral,
					)
					.clicked()
					{
						if self.picker {
							self.page = Page::Choose;
						} else {
							close = true;
						}
					}
				});
			});
		if close || response.close {
			*self = Self::default();
		}
	}
	fn show_picker(&mut self, ctx: &egui::Context, state: &mut State, commands: &mut Vec<Command>) {
		if self.generation != Some(state.generation) {
			*self = Self::default();
			return;
		}
		if self.submitted
			&& state
				.guild_creation
				.result
				.as_ref()
				.is_some_and(|result| result.as_ref().is_ok_and(|id| state.guild(*id).is_some()))
		{
			*self = Self::default();
			return;
		}
		let (title, subtitle) = match self.page {
			Page::Choose => (
				"Create Your Server",
				"Your server is where you and your friends hang out. Make yours and start talking.",
			),
			Page::Audience => (
				"Tell Us More About Your Server",
				"Is your new server for a few friends or a larger community?",
			),
			Page::Customize => (
				"Customize Your Server",
				"Give your new server a name and icon. You can change them later.",
			),
			Page::Join => unreachable!(),
		};
		let response = dialog::Dialog::new("add-server-dialog", title)
			.subtitle(subtitle)
			.width(460.0)
			.show(ctx, |d| match self.page {
				Page::Choose => self.choose(d, state),
				Page::Audience => self.audience(d, state),
				Page::Customize => self.customize(d, state, commands),
				Page::Join => {}
			});
		if response.close {
			*self = Self::default();
		}
	}
	fn choose(&mut self, d: &mut dialog::Body<'_>, state: &State) {
		d.scroll(250.0, |ui| {
			ui.spacing_mut().item_spacing.y = 10.0;
			if option_row(ui, icons::Icon::Plus, "Create My Own").clicked() {
				self.page = Page::Audience;
			}
			ui.add_space(8.0);
			ui.label(design::eyebrow(
				ui,
				"Have an invite already?",
				design::palette(ui).muted,
			));
			if option_row(ui, icons::Icon::Compass, "Join a Server").clicked() {
				self.page = Page::Join;
				self.focus = true;
			}
			if state.demo {
				ui.add_space(4.0);
				design::notice(
					ui,
					design::Level::Info,
					"Offline preview - creating and joining servers are disabled.",
				);
			}
		});
	}
	fn audience(&mut self, d: &mut dialog::Body<'_>, state: &State) {
		d.content(|ui| {
			ui.spacing_mut().item_spacing.y = 10.0;
			let community = option_row(ui, icons::Icon::Globe, "For a club or community").clicked();
			let friends = option_row(ui, icons::Icon::People, "For me and my friends").clicked();
			if community || friends {
				self.begin_customize(state);
			}
			ui.add_space(8.0);
			ui.horizontal_wrapped(|ui| {
				ui.label("Not sure?");
				if ui.link("Skip this question").clicked() {
					self.begin_customize(state);
				}
				ui.label("for now.");
			});
		});
		d.footer(|ui| {
			if dialog::action(ui, "Back", dialog::Action::Neutral).clicked() {
				self.page = Page::Choose;
			}
		});
	}
	fn begin_customize(&mut self, state: &State) {
		if self.name.is_empty() {
			self.name = state
				.user
				.as_ref()
				.map(|user| format!("{}'s server", user.name))
				.unwrap_or_else(|| "My server".to_owned());
		}
		self.page = Page::Customize;
		self.name_focus = true;
	}
	fn customize(
		&mut self,
		d: &mut dialog::Body<'_>,
		state: &mut State,
		commands: &mut Vec<Command>,
	) {
		d.scroll(275.0, |ui| {
			let colors = design::palette(ui);
			ui.vertical_centered(|ui| {
				let (rect, response) =
					ui.allocate_exact_size(egui::Vec2::splat(82.0), egui::Sense::click());
				response.widget_info(|| {
					egui::WidgetInfo::labeled(
						egui::Role::Button,
						!self.icon_pending,
						if self.icon.is_some() {
							"Change server icon"
						} else {
							"Upload server icon"
						},
					)
				});
				ui.painter().circle(
					rect.center(),
					40.0,
					colors.base,
					egui::Stroke::new(
						2.0,
						if response.hovered() || response.has_focus() {
							colors.accent
						} else {
							colors.border
						},
					),
				);
				if let Some(texture) = &self.icon_preview {
					ui.painter().image(
						texture.id(),
						rect.shrink(5.0),
						egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
						egui::Color32::WHITE,
					);
				} else {
					icons::paint(
						ui.painter(),
						icons::Icon::Image,
						rect.shrink(27.0),
						colors.muted,
					);
				}
				if response.clicked() && !self.icon_pending {
					self.icon_requested = true;
					self.icon_pending = true;
					self.icon_error = None;
				}
			});
			ui.add_space(18.0);
			let label = design::label(ui, "Server name");
			let input = design::input(
				ui,
				egui::TextEdit::singleline(&mut self.name)
					.char_limit(100)
					.hint_text("My server"),
			)
			.labelled_by(label.id);
			if std::mem::take(&mut self.name_focus) {
				input.request_focus();
			}
			if input.changed() {
				self.status = "";
			}
			if let Some(error) = self.icon_error {
				ui.add_space(10.0);
				design::notice(ui, design::Level::Error, error);
			}
			let result = self
				.submitted
				.then_some(state.guild_creation.result.as_ref())
				.flatten();
			if let Some(Err(error)) = result {
				ui.add_space(10.0);
				design::notice(ui, design::Level::Error, error.label());
			} else if result.is_some_and(Result::is_ok) {
				ui.add_space(10.0);
				design::notice(
					ui,
					design::Level::Success,
					"Server created. Waiting for Discord to add it to your server list.",
				);
			} else if state.demo {
				ui.add_space(10.0);
				design::notice(
					ui,
					design::Level::Info,
					"Offline preview - creation is disabled.",
				);
			} else if !self.status.is_empty() {
				ui.add_space(10.0);
				design::notice(ui, design::Level::Error, self.status);
			}
		});
		d.footer(|ui| {
			let accepted = self.submitted
				&& state
					.guild_creation
					.result
					.as_ref()
					.is_some_and(Result::is_ok);
			let busy = self.icon_pending || state.guild_creation.pending;
			let name = self.name.trim();
			let valid =
				(2..=100).contains(&name.chars().count()) && !name.chars().any(char::is_control);
			ui.add_enabled_ui(!state.demo && !busy && !accepted && valid, |ui| {
				if dialog::action(
					ui,
					if busy { "Please wait..." } else { "Create" },
					dialog::Action::Primary,
				)
				.clicked()
				{
					match state.create_guild(name.to_owned(), self.icon.clone()) {
						Some(command) => {
							commands.push(command);
							self.submitted = true;
							self.status = "";
						}
						None => self.status = "Unable to create this server right now.",
					}
				}
			});
			if dialog::action(ui, "Back", dialog::Action::Neutral).clicked() {
				self.page = Page::Audience;
			}
		});
	}

	/// Field, examples and the resolved invite preview; returns the lookup state for the footer.
	fn body(
		&mut self,
		ui: &mut egui::Ui,
		state: &State,
		avatars: &mut crate::avatars::Avatars,
	) -> (bool, bool, bool, bool, Option<String>) {
		let colors = design::palette(ui);
		let label = ui.label(design::eyebrow(ui, "Invite link", colors.muted));
		ui.add_space(6.0);
		let input = invite_input(ui, &mut self.input, std::mem::take(&mut self.focus))
			.labelled_by(label.id);
		if input.changed() {
			self.status = "";
		}
		ui.add_space(10.0);
		ui.add(
			egui::Label::new(
				egui::RichText::new("Invites look like")
					.size(12.0)
					.color(colors.muted),
			)
			.wrap(),
		);
		ui.add_space(2.0);
		ui.add(
			egui::Label::new(
				egui::RichText::new("hTKzmak · discord.gg/hTKzmak · discord.gg/wumpus-friends")
					.size(12.0)
					.monospace()
					.color(colors.muted),
			)
			.wrap(),
		);
		let parsed = input_code(&self.input);
		let entry = parsed
			.as_ref()
			.and_then(|code| state.invites.get(code))
			.filter(|(at, _)| at.elapsed().as_secs() < 300);
		let loading = entry.is_some_and(|(_, value)| value.is_none());
		let preview = entry
			.and_then(|(_, value)| value.as_ref())
			.and_then(|value| value.as_ref().ok());
		let lookup_error = entry
			.and_then(|(_, value)| value.as_ref())
			.and_then(|value| value.as_ref().err());
		let member = preview.is_some_and(|preview| state.guild(preview.guild).is_some());
		let ready = preview.is_some();
		let accepted = parsed.as_deref() == Some(state.invite_join.code.as_str())
			&& matches!(state.invite_join.result, Some(Ok(_)));
		let join_error = (parsed.as_deref() == Some(state.invite_join.code.as_str()))
			.then_some(state.invite_join.result.as_ref())
			.flatten()
			.and_then(|result| result.as_ref().err());
		if loading || preview.is_some() {
			ui.add_space(16.0);
			egui::Frame::new()
				.fill(colors.base)
				.corner_radius(10)
				.inner_margin(14)
				.show(ui, |ui| {
					ui.set_width(ui.available_width());
					ui.horizontal(|ui| {
						ui.spacing_mut().item_spacing.x = 14.0;
						let (icon, _) =
							ui.allocate_exact_size(egui::Vec2::splat(52.0), egui::Sense::hover());
						match preview.and_then(|p| p.embed.thumbnail.as_ref()) {
							Some(media) => {
								let mut icon_ui =
									ui.new_child(egui::UiBuilder::new().max_rect(icon));
								avatars.show_media(
									&mut icon_ui,
									media,
									icon.size(),
									state.demo,
									crate::avatars::Surface::Inline,
								);
							}
							None => {
								ui.painter().rect_filled(icon, 16, colors.raised);
								let initial = preview
									.and_then(|p| p.embed.title.as_deref())
									.and_then(|title| title.chars().next())
									.map(|c| c.to_uppercase().to_string());
								ui.painter().text(
									icon.center(),
									egui::Align2::CENTER_CENTER,
									initial.as_deref().unwrap_or("?"),
									egui::FontId::proportional(20.0),
									colors.muted,
								);
							}
						}
						ui.vertical(|ui| {
							ui.spacing_mut().item_spacing.y = 4.0;
							ui.add(
								egui::Label::new(
									design::semibold(
										ui,
										preview
											.and_then(|p| p.embed.title.as_deref())
											.unwrap_or("Checking invite…"),
										17.0,
									)
									.color(colors.text_strong),
								)
								.truncate(),
							);
							match preview
								.and_then(|p| p.embed.description.as_deref())
								.and_then(crate::invites::counts)
							{
								Some((online, members)) => {
									ui.horizontal(|ui| {
										ui.spacing_mut().item_spacing.x = 6.0;
										crate::invites::dot_stat(
											ui,
											colors.positive,
											online,
											"Online",
										);
										ui.add_space(6.0);
										crate::invites::dot_stat(
											ui,
											colors.muted,
											members,
											"Members",
										);
									});
								}
								None => {
									ui.add(
										egui::Label::new(
											egui::RichText::new(if member {
												"You are already a member."
											} else if loading {
												"Fetching server details…"
											} else {
												"Review this server, then choose Join Server."
											})
											.size(13.0)
											.color(colors.muted),
										)
										.truncate(),
									);
								}
							}
							if preview.is_some() {
								ui.add(
									egui::Label::new(
										egui::RichText::new(if member {
											"You are already a member of this server."
										} else {
											"Choose Join Server to confirm."
										})
										.size(12.0)
										.color(colors.muted),
									)
									.wrap(),
								);
							}
						});
					});
				});
		}
		for (text, danger) in [
			(lookup_error.map(|error| error.label()), true),
			(join_error.map(|error| error.label()), true),
			(
				accepted.then_some(
					"Invite accepted. Waiting for server access; complete any server rules in Discord.",
				),
				false,
			),
			(
				state
					.demo
					.then_some("Offline preview — joining servers is disabled."),
				false,
			),
			((!self.status.is_empty()).then_some(self.status), false),
		] {
			let Some(text) = text else { continue };
			ui.add_space(12.0);
			let (fill, color) = if danger {
				(colors.danger.gamma_multiply(0.14), colors.danger)
			} else {
				(colors.base, colors.muted)
			};
			egui::Frame::new()
				.fill(fill)
				.corner_radius(8)
				.inner_margin(egui::Margin::symmetric(12, 10))
				.show(ui, |ui| {
					ui.set_width(ui.available_width());
					ui.add(
						egui::Label::new(egui::RichText::new(text).size(13.0).color(color)).wrap(),
					);
				});
		}
		ui.add_space(16.0);
		egui::Frame::new()
			.fill(colors.base)
			.corner_radius(10)
			.inner_margin(14)
			.show(ui, |ui| {
				ui.set_width(ui.available_width());
				ui.spacing_mut().item_spacing.y = 4.0;
				ui.label(design::semibold(ui, "Don't have an invite?", 15.0));
				ui.hyperlink_to(
					egui::RichText::new("Explore discoverable communities in Discord ↗")
						.size(13.0)
						.color(colors.link),
					"https://discord.com/servers",
				);
			});
		(ready, member, loading, accepted, parsed)
	}

	/// Look the invite up, or join once a preview is on screen; never both in one click.
	fn submit(
		&mut self,
		state: &mut State,
		parsed: Option<String>,
		ready: bool,
		commands: &mut Vec<Command>,
	) {
		let Some(code) = parsed else {
			self.status = "Enter a valid Discord invite link or invite code.";
			return;
		};
		let command = if ready {
			state.join_invite(code)
		} else {
			// Only an explicit retry may discard a completed failed lookup.
			state.invites.remove(&code);
			state.request_join_preview(code)
		};
		match command {
			Some(command) => {
				commands.push(command);
				self.status = "";
			}
			None => {
				self.status = "Unable to proceed. Check your connection or try again after the current request.";
			}
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn invite_field_centers_hint_and_text_with_padding_and_focus_outline() {
		for light in [false, true] {
			for width in [240.0, 490.0] {
				for initial in ["", "synthetic-invite"] {
					let ctx = egui::Context::default();
					design::apply(&ctx);
					ctx.set_visuals(if light {
						egui::Visuals::light()
					} else {
						egui::Visuals::dark()
					});
					let mut text = initial.to_owned();
					let mut rect = egui::Rect::NOTHING;
					for _ in 0..2 {
						let output = ctx.run_ui(egui::RawInput::default(), |ui| {
							ui.set_width(width);
							let response = invite_input(ui, &mut text, true);
							assert!(response.has_focus());
							rect = response.rect;
						});
						let mut shapes = Vec::new();
						fn flatten<'a>(shape: &'a egui::Shape, out: &mut Vec<&'a egui::Shape>) {
							if let egui::Shape::Vec(children) = shape {
								for child in children {
									flatten(child, out);
								}
							} else {
								out.push(shape);
							}
						}
						for shape in &output.shapes {
							flatten(&shape.shape, &mut shapes);
						}
						assert!((rect.height() - 48.0).abs() < 1.0, "{rect:?}");
						let label = shapes
							.iter()
							.find_map(|shape| match shape {
								egui::Shape::Text(t)
									if t.galley.job.text
										== if initial.is_empty() {
											"https://discord.gg/hTKzmak"
										} else {
											initial
										} =>
								{
									Some(t.galley.rect.translate(t.pos.to_vec2()))
								}
								_ => None,
							})
							.expect("input text is rendered");
						assert!(
							(label.center().y - rect.center().y).abs() <= 1.0,
							"text {label:?}, field {rect:?}"
						);
						assert!(label.left() >= rect.left() + 11.0);
						assert!(shapes.iter().any(|shape| matches!(shape, egui::Shape::Rect(r) if r.rect == rect && r.stroke.width == 2.0 && r.stroke.color == design::palette_for(&ctx).accent)));
						output.drop_without_applying_deltas();
					}
				}
			}
		}
	}
	fn frame(
		ctx: &egui::Context,
		dialog: &mut JoinDialog,
		state: &mut State,
		commands: &mut Vec<Command>,
		size: egui::Vec2,
		events: Vec<egui::Event>,
	) -> Vec<(String, egui::Rect)> {
		let mut avatars = crate::avatars::Avatars::default();
		fn labels(shape: &egui::Shape, output: &mut Vec<(String, egui::Rect)>) {
			match shape {
				egui::Shape::Text(text) => output.push((
					text.galley.job.text.clone(),
					text.galley.rect.translate(text.pos.to_vec2()),
				)),
				egui::Shape::Vec(shapes) => {
					for shape in shapes {
						labels(shape, output);
					}
				}
				_ => {}
			}
		}
		let output = ctx.run_ui(
			egui::RawInput {
				screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
				events,
				..Default::default()
			},
			|ui| dialog.show(ui.ctx(), state, &mut avatars, commands),
		);
		let mut texts = vec![];
		for shape in &output.shapes {
			labels(&shape.shape, &mut texts);
		}
		output.drop_without_applying_deltas();
		texts
	}
	fn click(
		ctx: &egui::Context,
		dialog: &mut JoinDialog,
		state: &mut State,
		commands: &mut Vec<Command>,
		size: egui::Vec2,
		label: &str,
	) {
		let texts = frame(ctx, dialog, state, commands, size, vec![]);
		let rect = texts
			.iter()
			.find(|(text, _)| text == label)
			.unwrap_or_else(|| panic!("Missing {label}: {texts:?}"))
			.1;
		assert!(egui::Rect::from_min_size(egui::Pos2::ZERO, size).contains_rect(rect));
		for pressed in [true, false] {
			frame(
				ctx,
				dialog,
				state,
				commands,
				size,
				vec![
					egui::Event::PointerMoved(rect.center()),
					egui::Event::PointerButton {
						pos: rect.center(),
						button: egui::PointerButton::Primary,
						pressed,
						modifiers: egui::Modifiers::NONE,
					},
				],
			);
		}
	}
	#[test]
	fn add_server_flow_reaches_creation_and_preserves_the_draft_on_back() {
		let ctx = egui::Context::default();
		design::apply(&ctx);
		let size = egui::vec2(320.0, 760.0);
		let mut state = State {
			auth: client_core::auth::AuthState::Authenticated,
			gateway_connected: true,
			..State::default()
		};
		let mut dialog = JoinDialog::default();
		dialog.open_picker(state.generation);
		let mut commands = vec![];
		for _ in 0..3 {
			frame(&ctx, &mut dialog, &mut state, &mut commands, size, vec![]);
		}
		click(
			&ctx,
			&mut dialog,
			&mut state,
			&mut commands,
			size,
			"Create My Own",
		);
		assert_eq!(dialog.page, Page::Audience);
		click(
			&ctx,
			&mut dialog,
			&mut state,
			&mut commands,
			size,
			"For me and my friends",
		);
		assert_eq!(dialog.page, Page::Customize);
		dialog.name = "Synthetic server".into();
		click(&ctx, &mut dialog, &mut state, &mut commands, size, "Back");
		assert_eq!(dialog.page, Page::Audience);
		dialog.begin_customize(&state);
		assert_eq!(dialog.name, "Synthetic server");
		click(&ctx, &mut dialog, &mut state, &mut commands, size, "Create");
		let Some(Command::CreateGuild { request, .. }) = commands.pop() else {
			panic!("create command missing")
		};
		assert_eq!(request.name(), "Synthetic server");
		assert!(request.icon().is_none());
		assert!(dialog.submitted);
		assert!(state.guild_creation.pending);
	}
	#[test]
	fn join_dialog_checks_then_confirms_once_and_clears_on_session_change() {
		for (light, size) in [
			(false, egui::vec2(960.0, 760.0)),
			(true, egui::vec2(320.0, 760.0)),
		] {
			let ctx = egui::Context::default();
			design::apply(&ctx);
			ctx.set_visuals(if light {
				egui::Visuals::light()
			} else {
				egui::Visuals::dark()
			});
			let mut state = State {
				auth: client_core::auth::AuthState::Authenticated,
				gateway_connected: true,
				..State::default()
			};
			let mut dialog = JoinDialog::default();
			dialog.open(state.generation);
			dialog.input = "https://discord.gg/synthetic".into();
			let mut commands = vec![];
			for _ in 0..3 {
				frame(&ctx, &mut dialog, &mut state, &mut commands, size, vec![]);
			}
			assert!(commands.is_empty());
			click(
				&ctx,
				&mut dialog,
				&mut state,
				&mut commands,
				size,
				"Check Invite",
			);
			assert!(matches!(commands.pop(), Some(Command::Invite { .. })));
			frame(&ctx, &mut dialog, &mut state, &mut commands, size, vec![]);
			assert!(commands.is_empty());
			state.apply_invite(
				"synthetic".into(),
				Err(client_core::auth::Failure::Forbidden),
			);
			let texts = frame(&ctx, &mut dialog, &mut state, &mut commands, size, vec![]);
			assert!(
				texts.iter().any(|(text, _)| text == "Permission denied"),
				"{texts:?}"
			);
			assert!(commands.is_empty(), "no automatic retries");
			click(
				&ctx,
				&mut dialog,
				&mut state,
				&mut commands,
				size,
				"Check Invite",
			);
			assert!(matches!(commands.pop(), Some(Command::Invite { .. })));
			state.apply_invite(
				"synthetic".into(),
				Ok(model::InvitePreview {
					guild: model::Id(12),
					embed: model::Embed {
						title: Some("Synthetic server".into()),
						..Default::default()
					},
				}),
			);
			for _ in 0..2 {
				frame(&ctx, &mut dialog, &mut state, &mut commands, size, vec![]);
			}
			assert!(commands.is_empty(), "preview never joins automatically");
			click(
				&ctx,
				&mut dialog,
				&mut state,
				&mut commands,
				size,
				"Join Server",
			);
			let Some(Command::JoinInvite { request, .. }) = commands.pop() else {
				panic!("join missing")
			};
			click(
				&ctx,
				&mut dialog,
				&mut state,
				&mut commands,
				size,
				"Please wait…",
			);
			assert!(commands.is_empty());
			state.apply(client_core::Envelope {
				generation: state.generation,
				event: client_core::Event::JoinInvite {
					request,
					result: Ok(model::Id(12)),
				},
			});
			assert!(state.guild(model::Id(12)).is_none());
			// The dialog now closes from the header control or Escape; there is no Back button.
			assert!(!texts.iter().any(|(text, _)| text == "Back"));
			frame(
				&ctx,
				&mut dialog,
				&mut state,
				&mut commands,
				size,
				vec![egui::Event::Key {
					key: egui::Key::Escape,
					physical_key: None,
					pressed: true,
					repeat: false,
					modifiers: egui::Modifiers::NONE,
				}],
			);
			assert!(dialog.generation.is_none());
			dialog.open(state.generation);
			state.generation += 1;
			frame(&ctx, &mut dialog, &mut state, &mut commands, size, vec![]);
			assert!(dialog.generation.is_none());
		}
	}
}
