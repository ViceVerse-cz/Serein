//! Native component controls; transport authorization and validation stay in client-core.
use crate::{avatars::Avatars, design, dialog, markdown};
use client_core::{Command, State};
use model::{Component, Id, Message};

pub(crate) type Action = (Id, String, Vec<String>);

pub(crate) struct MediaUi<'a> {
	pub component_viewing: &'a mut Option<(Id, u64)>,
	pub viewing: &'a mut Option<(Id, Id)>,
	pub download: &'a mut crate::attachments::DownloadUi,
	pub audio: &'a mut crate::audio::AudioUi,
	pub video: &'a mut crate::video::VideoUi,
}

#[derive(Default)]
pub(crate) struct Components {
	// One active select draft, bounded by the protocol's 25 values; edits replace its identity.
	generation: u64,
	channel: Option<Id>,
	revealed: std::collections::BTreeSet<u64>,
	text_revealed: std::collections::BTreeMap<u64, u32>,
	pub(crate) search: crate::member_search::Search,
	pub(crate) remote_search: bool,
	select: Option<(egui::Id, Vec<String>)>,
	modal: Option<(u64, Id, Vec<Component>)>,
	formatted: markdown::FormatCache,
	pub(crate) file_request: Option<String>,
	files: Vec<(String, Vec<(usize, String)>)>,
	// True while rendering a section accessory, so media draws as a compact thumbnail.
	accessory: bool,
}

impl Components {
	pub(crate) fn forget_message(&mut self, id: Id) {
		self.formatted.retain(|message| message != id);
		self.revealed.clear();
		self.text_revealed.clear();
		self.select = None;
		self.search = Default::default();
		self.remote_search = false;
	}
	pub(crate) fn modal_id(&self) -> Option<Id> {
		self.modal.as_ref().map(|(_, id, _)| *id)
	}
	pub(crate) fn set_files(&mut self, custom_id: &str, files: Vec<(usize, String)>) {
		if self.modal.is_none()
			|| custom_id.chars().count() > 100
			|| custom_id.len() > 400
			|| files.len() > 10
			|| files.iter().any(|(_, name)| name.len() > 1024)
		{
			return;
		}
		self.files.retain(|(key, _)| key != custom_id);
		if self.files.len() < 5 {
			self.files.push((custom_id.to_owned(), files));
		}
	}
	pub(crate) fn show(
		&mut self,
		ui: &mut egui::Ui,
		message: &Message,
		state: &State,
		avatars: &mut Avatars,
		opening: &mut Option<String>,
		media_ui: &mut MediaUi<'_>,
	) -> Option<Action> {
		if self.generation != state.generation || self.channel != state.selected {
			self.generation = state.generation;
			self.channel = state.selected;
			self.remote_search = false;
			self.revealed.clear();
			self.text_revealed.clear();
			self.select = None;
			self.search = Default::default();
			self.formatted = Default::default();
			self.files.clear();
			self.file_request = None;
		}
		let mut action = None;
		let identity = egui::Id::unique((
			state.generation,
			message.channel,
			message.id,
			&message.components,
		));
		let enabled = !message.forwarded
			&& !state.interactions.busy()
			&& state.gateway_connected
			&& state.freshness == model::Freshness::Fresh
			&& state.can_view(message.channel);
		ui.push_id(identity, |ui| {
			// Component trees keep Discord's message content width instead of the viewport.
			ui.set_max_width(ui.available_width().min(COMPONENTS_MAX_WIDTH));
			for (index, component) in message.components.iter().enumerate() {
				self.show_component(
					ui,
					component,
					identity.with(index),
					message,
					state,
					avatars,
					opening,
					enabled,
					&mut action,
					media_ui,
				);
			}
		});
		action
	}

	#[allow(clippy::too_many_arguments)]
	fn show_component(
		&mut self,
		ui: &mut egui::Ui,
		c: &Component,
		id: egui::Id,
		message: &Message,
		state: &State,
		avatars: &mut Avatars,
		opening: &mut Option<String>,
		enabled: bool,
		action: &mut Option<Action>,
		media_ui: &mut MediaUi<'_>,
	) {
		let colors = design::palette(ui);
		ui.push_id(id, |ui| {
			if c.spoiler {
				let revealed = self.revealed.contains(&id.value());
				if !revealed {
					if ui.button("Reveal spoiler component").clicked() {
						if self.revealed.len() >= 256 {
							self.revealed.clear();
						}
						self.revealed.insert(id.value());
					}
					return;
				}
			}
			match c.kind {
				1 | 9 | 17 => {
					let frame = if c.kind == 17 {
						egui::Frame::new()
							.fill(colors.raised)
							.stroke(egui::Stroke::new(
								1.0,
								c.accent_color
									.map(|rgb| {
										egui::Color32::from_rgb(
											(rgb >> 16) as u8,
											(rgb >> 8) as u8,
											rgb as u8,
										)
									})
									.unwrap_or(colors.border),
							))
							.corner_radius(8)
							.inner_margin(12)
					} else {
						egui::Frame::new()
					};
					frame.show(ui, |ui| {
						if c.kind != 1 {
							// Discord separates stacked container/section children by 8px.
							ui.spacing_mut().item_spacing.y = 8.0;
						}
						if c.kind == 1 {
							ui.horizontal_wrapped(|ui| {
								for (index, child) in c.components.iter().enumerate() {
									self.show_component(
										ui,
										child,
										id.with(index),
										message,
										state,
										avatars,
										opening,
										enabled,
										action,
										media_ui,
									);
								}
							});
						} else if c.kind == 9 {
							// The accessory takes its natural width at the right edge first;
							// the text column wraps in whatever remains.
							ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
								ui.spacing_mut().item_spacing.x = 16.0;
								if let Some(accessory) = &c.accessory {
									let thumbnail = accessory.kind == 11;
									let accessory_width = if thumbnail {
										THUMBNAIL_SIZE
									} else {
										(ui.available_width() * 0.45).max(40.0)
									};
									ui.scope(|ui| {
										ui.set_max_width(accessory_width);
										self.accessory = true;
										self.show_component(
											ui,
											accessory,
											id.with("accessory"),
											message,
											state,
											avatars,
											opening,
											enabled,
											action,
											media_ui,
										);
										self.accessory = false;
									});
								}
								ui.allocate_ui_with_layout(
									egui::vec2(ui.available_width().max(40.0), 0.0),
									egui::Layout::top_down(egui::Align::Min),
									|ui| {
										ui.spacing_mut().item_spacing.y = 8.0;
										for (index, child) in c.components.iter().enumerate() {
											self.show_component(
												ui,
												child,
												id.with(index),
												message,
												state,
												avatars,
												opening,
												enabled,
												action,
												media_ui,
											);
										}
									},
								);
							});
						} else {
							for (index, child) in c.components.iter().enumerate() {
								self.show_component(
									ui,
									child,
									id.with(index),
									message,
									state,
									avatars,
									opening,
									enabled,
									action,
									media_ui,
								);
							}
						}
						if c.kind != 9
							&& let Some(accessory) = &c.accessory
						{
							self.show_component(
								ui,
								accessory,
								id.with("accessory"),
								message,
								state,
								avatars,
								opening,
								enabled,
								action,
								media_ui,
							);
						}
					});
				}
				2 => {
					if c.style == Some(6) {
						return;
					}
					let label = component_label(c);
					let linked = c.style == Some(5);
					let text = egui::RichText::new(label);
					let text = if matches!(c.style, Some(1 | 3 | 4)) {
						text.color(colors.accent_text)
					} else {
						text
					};
					let image = c
						.emoji
						.as_ref()
						.and_then(|emoji| emoji.id)
						.and_then(|id| avatars.custom_image(ui.ctx(), id, 18.0, state.demo));
					let mut button = if let Some(image) = image {
						egui::Button::image_and_text(image, text)
							.image_tint_follows_text_color(false)
					} else {
						egui::Button::new(text)
					}
					.wrap_mode(if self.accessory {
						egui::TextWrapMode::Truncate
					} else {
						egui::TextWrapMode::Extend
					})
					.min_size(egui::vec2(0.0, 32.0))
					.corner_radius(6)
					.stroke(egui::Stroke::new(1.0, colors.border));
					if linked {
						button = button.right_text("   ");
					}
					button = match c.style {
						Some(1) => button.fill(colors.accent),
						Some(3) => button.fill(colors.positive),
						Some(4) => button.fill(colors.danger),
						_ => button,
					};
					let response = ui.add_enabled(!c.disabled && (linked || enabled), button);
					if linked {
						crate::icons::paint(
							ui.painter(),
							crate::icons::Icon::External,
							egui::Rect::from_center_size(
								response.rect.right_center() - egui::vec2(14.0, 0.0),
								egui::Vec2::splat(16.0),
							),
							colors.muted,
						);
					}
					if response.clicked() {
						if linked {
							*opening = c.url.as_deref().and_then(markdown::external_url);
						} else if let Some(custom_id) = &c.custom_id {
							*action = Some((message.id, custom_id.clone(), vec![]));
						}
					}
				}
				3 | 5..=8 => {
					let mut values = self
						.select
						.as_ref()
						.filter(|(key, _)| *key == id)
						.map(|(_, values)| values.clone())
						.unwrap_or_else(|| default_values(c));
					ui.add_enabled_ui(enabled && !c.disabled, |ui| {
						let changed = select(
							ui,
							c,
							&mut values,
							state,
							message.channel,
							&mut self.search.text,
							&mut self.remote_search,
							avatars,
						);
						let valid = values.len() >= usize::from(c.min_values.unwrap_or(1))
							&& values.len() <= usize::from(c.max_values.unwrap_or(1));
						if changed {
							self.select = Some((id, values.clone()));
						}
						let submit = if c.max_values.unwrap_or(1) == 1 {
							changed && valid
						} else {
							ui.add_enabled(valid, egui::Button::new("Submit selection"))
								.clicked()
						};
						if submit && let Some(custom_id) = &c.custom_id {
							*action = Some((message.id, custom_id.clone(), values));
						}
					});
				}
				10 => {
					if self.text_revealed.len() >= 256
						&& !self.text_revealed.contains_key(&id.value())
					{
						self.text_revealed.clear();
					}
					let revealed = self.text_revealed.entry(id.value()).or_default();
					let mut surface = crate::select::Surface::new(ui, "component-text");
					let source = crate::mentions::MentionSource {
						state,
						channel: message.channel,
					};
					self.formatted
						.get_part(
							message.id,
							(id.value() % u64::from(u16::MAX)) as u16,
							c.content.as_deref().unwrap_or_default(),
						)
						.show_references(
							ui,
							opening,
							&message.mentions,
							Some(&source),
							&mut crate::profiles::ProfileSession::default(),
							(
								&state.channels,
								&mut None,
								&state.guilds,
								crate::mentions::known_roles(state, message.channel),
							),
							(avatars, state.demo, revealed),
							&mut surface,
						);
					surface.finish(ui);
				}
				11 => {
					if let Some(media) = &c.media {
						// A section accessory renders as a compact square thumbnail without controls.
						let thumbnail = self.accessory;
						show_media(
							ui,
							media,
							c.description.as_deref().filter(|_| !thumbnail),
							avatars,
							state.demo,
							opening,
							message,
							media_ui,
							&mut self.revealed,
							thumbnail,
						);
					}
				}
				12 => {
					for (index, item) in c.items.iter().enumerate() {
						let key = id.with(("gallery", index));
						let revealed = self.revealed.contains(&key.value());
						if item.spoiler && !revealed {
							if ui.button("Reveal spoiler media").clicked() {
								if self.revealed.len() >= 256 {
									self.revealed.clear();
								}
								self.revealed.insert(key.value());
							}
						} else {
							show_media(
								ui,
								&item.media,
								item.description.as_deref(),
								avatars,
								state.demo,
								opening,
								message,
								media_ui,
								&mut self.revealed,
								false,
							);
						}
					}
				}
				13 => {
					if let Some(file) = &c.file {
						show_media(
							ui,
							file,
							c.description.as_deref(),
							avatars,
							state.demo,
							opening,
							message,
							media_ui,
							&mut self.revealed,
							false,
						);
					}
				}
				14 => {
					ui.add_space(if c.spacing == Some(2) { 12.0 } else { 4.0 });
					if c.divider.unwrap_or(true) {
						ui.separator();
					}
				}
				_ => {
					ui.colored_label(
						colors.muted,
						format!("Unsupported component (type {})", c.kind),
					);
				}
			}
		});
	}

	pub(crate) fn dialogs(
		&mut self,
		ctx: &egui::Context,
		state: &mut State,
		commands: &mut Vec<Command>,
		avatars: &mut Avatars,
		opening: &mut Option<String>,
	) {
		let Some(modal) = state.interactions.modal.as_ref() else {
			if self.modal.take().is_some() {
				self.remote_search = false;
				self.search = Default::default();
			}
			self.files.clear();
			self.file_request = None;
			return;
		};
		if self
			.modal
			.as_ref()
			.is_none_or(|(generation, id, _)| *generation != state.generation || *id != modal.id)
		{
			let mut components = modal.components.clone();
			initialize(&mut components);
			self.files.clear();
			self.text_revealed.clear();
			self.search = Default::default();
			self.remote_search = false;
			self.modal = Some((state.generation, modal.id, components));
		}
		let mut submit = false;
		let busy = state.interactions.busy();
		let components = &mut self.modal.as_mut().expect("modal initialized").2;
		let response = dialog::Dialog::new(
			("application-modal", state.generation, modal.id),
			&modal.title,
		)
		.width(520.0)
		.show(ctx, |dialog| {
			let mut valid = true;
			dialog.content(|ui| {
				ui.add_enabled_ui(!busy, |ui| {
					egui::ScrollArea::vertical()
						.max_height(440.0)
						.show(ui, |ui| {
							ui.spacing_mut().item_spacing.y = 10.0;
							for (index, component) in components.iter_mut().enumerate() {
								ui.push_id(index, |ui| {
									valid &= field(
										ui,
										component,
										state,
										&mut self.file_request,
										&self.files,
										&mut self.search.text,
										&mut self.remote_search,
										avatars,
										opening,
										&mut self.formatted,
										&mut self.text_revealed,
									);
								});
							}
						});
				});
				if let Some(error) = state.interactions.error {
					ui.colored_label(design::palette(ui).danger, error);
				}
				if busy {
					ui.label("Submitting…");
				}
			});
			dialog.footer(|ui| {
				ui.add_enabled_ui(valid && !busy, |ui| {
					submit = dialog::action(ui, "Submit", dialog::Action::Primary).clicked();
				});
			});
		});
		let submitted = submit.then(|| components.clone());
		if response.close && !busy {
			state.dismiss_interaction_modal();
			self.modal = None;
		} else if let Some(submitted) = submitted
			&& let Some(command) = state.submit_interaction_modal(submitted)
		{
			commands.push(command);
		}
	}
}

fn component_label(c: &Component) -> String {
	let emoji = c
		.emoji
		.as_ref()
		.filter(|emoji| emoji.id.is_none())
		.and_then(|emoji| emoji.name.as_deref())
		.unwrap_or_default();
	let label = c.label.as_deref().unwrap_or("Button");
	if emoji.is_empty() {
		label.to_owned()
	} else {
		format!("{emoji} {label}")
	}
}
fn default_values(c: &Component) -> Vec<String> {
	if !c.values.is_empty() {
		c.values.clone()
	} else if !c.default_values.is_empty() {
		c.default_values
			.iter()
			.map(|value| value.id.to_string())
			.collect()
	} else {
		c.options
			.iter()
			.filter(|option| option.default)
			.map(|option| option.value.clone())
			.collect()
	}
}
fn initialize(components: &mut [Component]) {
	for c in components {
		if matches!(c.kind, 3 | 5..=8 | 19 | 21 | 22) {
			c.values = default_values(c);
			let limit = usize::from(c.max_values.unwrap_or(if c.kind == 22 {
				c.options.len() as u16
			} else {
				1
			}))
			.min(25);
			c.values.truncate(limit);
			let mut unique = std::collections::BTreeSet::new();
			c.values
				.retain(|value| value.len() <= 400 && unique.insert(value.clone()));
		}
		if c.kind == 23 {
			c.checked = Some(c.checked.unwrap_or(c.default));
		}
		if c.kind == 21
			&& let Some(value) = &c.value
		{
			c.values = vec![value.clone()];
		}
		initialize(&mut c.components);
		if let Some(child) = &mut c.component {
			initialize(std::slice::from_mut(child.as_mut()));
		}
	}
}
#[allow(clippy::too_many_arguments)]
fn select(
	ui: &mut egui::Ui,
	c: &Component,
	values: &mut Vec<String>,
	state: &State,
	channel: Id,
	query: &mut String,
	remote_search: &mut bool,
	avatars: &mut Avatars,
) -> bool {
	let mut changed = false;
	let colors = design::palette(ui);
	let users = if matches!(c.kind, 5 | 7) && !values.is_empty() {
		crate::mentions::known_users(state, channel)
	} else {
		Vec::new()
	};
	let selected_text = values
		.iter()
		.map(|value| {
			c.options
				.iter()
				.find(|option| option.value == *value)
				.map(|option| option.label.clone())
				.or_else(|| {
					value
						.parse::<Id>()
						.ok()
						.and_then(|id| state.channel(id))
						.map(|channel| channel.name.clone())
				})
				.or_else(|| {
					crate::mentions::known_roles(state, channel)
						.iter()
						.find(|role| Some(role.id) == value.parse::<Id>().ok())
						.map(|role| role.name.clone())
				})
				.or_else(|| {
					users
						.iter()
						.find(|user| Some(user.id) == value.parse::<Id>().ok())
						.map(|user| user.name.clone())
				})
				.unwrap_or_else(|| value.clone())
		})
		.collect::<Vec<_>>()
		.join(", ");
	let width = ui.available_width().min(400.0);
	egui::ComboBox::from_id_salt("selection")
		.width(width)
		.height(360.0)
		.selected_text(if selected_text.is_empty() {
			c.placeholder.as_deref().unwrap_or("Choose options")
		} else {
			&selected_text
		})
		.show_ui(ui, |ui| {
			ui.set_min_width((width - 16.0).max(40.0));
			if (c.min_values == Some(0) || !c.required)
				&& !values.is_empty()
				&& ui.button("Clear selection").clicked()
			{
				values.clear();
				changed = true;
				*remote_search = false;
				ui.close();
			}
			if (c.kind != 3 || c.options.len() > 10)
				&& ui
					.add(
						egui::TextEdit::singleline(query)
							.hint_text("Search options")
							.char_limit(64),
					)
					.changed()
			{
				*remote_search = matches!(c.kind, 5 | 7);
			}
			let filter = if c.kind == 3 && c.options.len() <= 10 {
				String::new()
			} else {
				query.to_lowercase()
			};
			// Keep at most 100 matching labels (64 KiB); search still traverses the available catalog.
			let mut options = Vec::<(String, String)>::new();
			let mut bytes = 0;
			let mut add = |value: String, label: &str| {
				if options.len() < 100
					&& bytes + value.len() + label.len() <= 64 * 1024
					&& label.to_lowercase().contains(&filter)
					&& !options.iter().any(|(key, _)| *key == value)
				{
					bytes += value.len() + label.len();
					options.push((value, label.to_owned()));
				}
			};
			for option in &c.options {
				add(option.value.clone(), &option.label);
			}
			if matches!(c.kind, 5 | 7) {
				for user in crate::mentions::known_users(state, channel) {
					add(user.id.to_string(), &user.name);
				}
				for view in &state.member_search {
					if view
						.request
						.as_ref()
						.is_some_and(|request| request.channel == channel)
					{
						for member in &view.rows {
							add(member.user.id.to_string(), &member.user.name);
						}
					}
				}
			}
			if matches!(c.kind, 6 | 7) {
				for role in crate::mentions::known_roles(state, channel) {
					add(role.id.to_string(), &role.name);
				}
			}
			if c.kind == 8 {
				let guild = state.channel(channel).and_then(|item| item.guild);
				for item in state.channels.iter().filter(|item| {
					item.guild == guild
						&& state.can_view(item.id)
						&& (c.channel_types.is_empty() || c.channel_types.contains(&item.kind))
				}) {
					add(item.id.to_string(), &item.name);
				}
			}
			for (value, label) in &options {
				let selected = values.contains(value);
				let option = c.options.iter().find(|option| option.value == *value);
				let mut job = egui::text::LayoutJob::default();
				job.append(
					&format!(
						"{}{}{}",
						if selected { "✓  " } else { "" },
						option
							.and_then(|o| o.emoji.as_ref())
							.filter(|e| e.id.is_none())
							.and_then(|e| e.name.as_ref())
							.map(|name| format!("{name} "))
							.unwrap_or_default(),
						label
					),
					0.0,
					egui::TextFormat {
						font_id: egui::FontId::proportional(14.0),
						color: colors.text,
						..Default::default()
					},
				);
				if let Some(description) = option.and_then(|option| option.description.as_deref()) {
					job.append(
						&format!("\n{description}"),
						0.0,
						egui::TextFormat {
							font_id: egui::FontId::proportional(12.0),
							color: colors.muted,
							..Default::default()
						},
					);
				}
				let image = option
					.and_then(|option| option.emoji.as_ref())
					.and_then(|emoji| emoji.id)
					.and_then(|id| avatars.custom_image(ui.ctx(), id, 24.0, state.demo));
				let button = if let Some(image) = image {
					egui::Button::image_and_text(image, job).image_tint_follows_text_color(false)
				} else {
					egui::Button::new(job)
				};
				if ui
					.add_enabled(
						c.max_values.unwrap_or(1) == 1
							|| selected || values.len() < usize::from(c.max_values.unwrap_or(1)),
						button
							.selected(selected)
							.min_size(egui::vec2(ui.available_width(), 40.0)),
					)
					.clicked()
				{
					if c.max_values.unwrap_or(1) == 1 {
						*values = vec![value.clone()];
						ui.close();
					} else if selected {
						values.retain(|existing| existing != value);
					} else {
						values.push(value.clone());
					}
					changed = true;
					*remote_search = false;
				}
			}
			if options.is_empty() {
				ui.label("No matching options loaded");
			}
			if options.len() == 100 {
				ui.small("Refine your search to see more results");
			}
		});
	if matches!(c.kind, 5..=8) {
		ui.small("Type to search members; available roles and channels are listed");
	}
	changed
}
#[allow(clippy::too_many_arguments)]
fn field(
	ui: &mut egui::Ui,
	c: &mut Component,
	state: &State,
	file_request: &mut Option<String>,
	files: &[(String, Vec<(usize, String)>)],
	query: &mut String,
	remote_search: &mut bool,
	avatars: &mut Avatars,
	opening: &mut Option<String>,
	formatted: &mut markdown::FormatCache,
	revealed: &mut std::collections::BTreeMap<u64, u32>,
) -> bool {
	let mut valid = true;
	ui.push_id((c.id, c.custom_id.clone()), |ui| {
		if let Some(label) = &c.label {
			ui.strong(label);
		}
		if let Some(description) = &c.description {
			ui.small(description);
		}
		match c.kind {
			1 | 18 => {
				for (index, child) in c.components.iter_mut().enumerate() {
					ui.push_id(index, |ui| {
						valid &= field(
							ui,
							child,
							state,
							file_request,
							files,
							query,
							remote_search,
							avatars,
							opening,
							formatted,
							revealed,
						);
					});
				}
				if let Some(child) = &mut c.component {
					valid &= field(
						ui,
						child,
						state,
						file_request,
						files,
						query,
						remote_search,
						avatars,
						opening,
						formatted,
						revealed,
					);
				}
			}
			4 => {
				let value = c.value.get_or_insert_with(String::new);
				let edit = if c.style == Some(2) {
					egui::TextEdit::multiline(value).desired_rows(5)
				} else {
					egui::TextEdit::singleline(value)
				};
				ui.add(
					edit.char_limit(usize::from(c.max_length.unwrap_or(4000)).min(4000))
						.font(egui::FontId::proportional(15.0))
						.margin(egui::vec2(12.0, 10.0))
						.hint_text(c.placeholder.as_deref().unwrap_or_default())
						.desired_width(f32::INFINITY),
				);
				let length = value.chars().count();
				valid = (!c.required && length == 0)
					|| (length >= usize::from(c.min_length.unwrap_or(u16::from(c.required)))
						&& length <= usize::from(c.max_length.unwrap_or(4000)));
			}
			3 | 5..=8 => {
				let mut values = std::mem::take(&mut c.values);
				select(
					ui,
					c,
					&mut values,
					state,
					state.selected.unwrap_or(Id(0)),
					query,
					remote_search,
					avatars,
				);
				valid = (!c.required && values.is_empty())
					|| (values.len() >= usize::from(c.min_values.unwrap_or(1))
						&& values.len() <= usize::from(c.max_values.unwrap_or(1)));
				c.values = values;
			}
			21 | 22 => {
				for option in &c.options {
					let chosen = c.values.contains(&option.value);
					let clicked = if c.kind == 21 {
						ui.radio(chosen, &option.label).clicked()
					} else {
						let mut selected = chosen;
						ui.checkbox(&mut selected, &option.label).changed()
					};
					if clicked {
						if chosen {
							c.values.retain(|value| value != &option.value);
						} else if c.kind == 21 {
							c.values = vec![option.value.clone()];
						} else if c.values.len()
							< usize::from(c.max_values.unwrap_or(c.options.len() as u16))
						{
							c.values.push(option.value.clone());
						}
					}
					if let Some(description) = &option.description {
						ui.label(
							egui::RichText::new(description)
								.small()
								.color(design::palette(ui).muted),
						);
					}
				}
				if c.kind == 21 {
					c.value = c.values.first().cloned();
				}
				valid = (!c.required && c.values.is_empty())
					|| (c.values.len() >= usize::from(c.min_values.unwrap_or(1))
						&& c.values.len()
							<= usize::from(c.max_values.unwrap_or(if c.kind == 21 {
								1
							} else {
								c.options.len() as u16
							})));
			}
			23 => {
				ui.checkbox(
					c.checked.get_or_insert(false),
					c.label.as_deref().unwrap_or("Confirm"),
				);
				valid = true;
			}
			10 => {
				let key = ui.scope_id().value();
				if revealed.len() >= 256 && !revealed.contains_key(&key) {
					revealed.clear();
				}
				let mut surface = crate::select::Surface::new(ui, "modal-markdown");
				let source = crate::mentions::MentionSource {
					state,
					channel: state.selected.unwrap_or(Id(0)),
				};
				formatted
					.get(Id(key), c.content.as_deref().unwrap_or_default())
					.show_references(
						ui,
						opening,
						&[],
						Some(&source),
						&mut crate::profiles::ProfileSession::default(),
						(&state.channels, &mut None, &state.guilds, &[]),
						(avatars, state.demo, revealed.entry(key).or_default()),
						&mut surface,
					);
				surface.finish(ui);
			}
			19 => {
				if !c.file_types.is_empty() {
					ui.small(format!("Allowed files: {}", c.file_types.join(", ")));
				}
				if ui.button("Choose files…").clicked() {
					*file_request = c.custom_id.clone();
				}
				if let Some((_, selected)) = files
					.iter()
					.find(|(key, _)| Some(key) == c.custom_id.as_ref())
				{
					c.values = selected
						.iter()
						.map(|(index, _)| index.to_string())
						.collect();
					for (_, name) in selected {
						ui.label(name);
					}
				}
				valid = (!c.required && c.values.is_empty())
					|| (c.values.len() >= usize::from(c.min_values.unwrap_or(1))
						&& c.values.len() <= usize::from(c.max_values.unwrap_or(1)));
			}
			_ => {
				ui.label(format!("Unsupported form field (type {}).", c.kind));
				valid = false;
			}
		}
	});
	valid
}
#[allow(clippy::too_many_arguments)]
/// Square side of a section thumbnail accessory, matching Discord's compact card art.
const THUMBNAIL_SIZE: f32 = 80.0;
/// Discord lays component trees out in the message content column, not across the viewport.
const COMPONENTS_MAX_WIDTH: f32 = 520.0;

#[allow(clippy::too_many_arguments)]
fn show_media(
	ui: &mut egui::Ui,
	media: &model::ComponentMedia,
	description: Option<&str>,
	avatars: &mut Avatars,
	demo: bool,
	opening: &mut Option<String>,
	message: &Message,
	media_ui: &mut MediaUi<'_>,
	revealed: &mut std::collections::BTreeSet<u64>,
	thumbnail: bool,
) {
	let url = resolve_media(&media.url, message);
	if let Some(attachment) = message.attachments.iter().find(|attachment| {
		media.url.strip_prefix("attachment://") == Some(attachment.filename.as_str())
			|| url.is_some_and(|url| {
				attachment.media.url.as_deref() == Some(url)
					|| attachment.media.proxy_url.as_deref() == Some(url)
			})
	}) {
		let reveal_id = egui::Id::unique((
			"component-file-spoiler",
			message.id,
			&message.components,
			attachment.id,
		))
		.value();
		if attachment.spoiler && !revealed.contains(&reveal_id) {
			if ui.button("Reveal spoiler attachment").clicked() {
				if revealed.len() >= 256 {
					revealed.clear();
				}
				revealed.insert(reveal_id);
			}
			return;
		}
		let previous_view = *media_ui.viewing;
		let mut surface = crate::select::Surface::new(ui, ("component-attachment", attachment.id));
		crate::attachments::show_subset(
			ui,
			message,
			std::slice::from_ref(attachment),
			avatars,
			media_ui.viewing,
			opening,
			media_ui.download,
			media_ui.audio,
			media_ui.video,
			demo,
			&mut surface,
		);
		surface.finish(ui);
		if *media_ui.viewing != previous_view
			|| media_ui
				.video
				.active
				.as_ref()
				.is_some_and(|(_, id, active)| *id == message.id && active == attachment)
		{
			*media_ui.component_viewing = Some((
				message.id,
				egui::Id::unique((&message.components, &message.attachments)).value(),
			));
		}
		if let Some(description) = description {
			ui.small(description);
		}
		return;
	}
	let image = model::EmbedMedia {
		url: resolve_media(&media.url, message).map(str::to_owned),
		proxy_url: media.proxy_url.clone(),
		width: media.width.unwrap_or(320),
		height: media.height.unwrap_or(180),
		..Default::default()
	};
	if thumbnail {
		let response = avatars
			.show_embed(ui, &image, egui::vec2(THUMBNAIL_SIZE, THUMBNAIL_SIZE), demo)
			.on_hover_cursor(egui::CursorIcon::PointingHand);
		if response.clicked() {
			*opening = resolve_media(&media.url, message).and_then(markdown::external_url);
		}
		return;
	}
	avatars.show_embed(
		ui,
		&image,
		egui::vec2(ui.available_width().min(420.0), 280.0),
		demo,
	);
	if let Some(description) = description {
		ui.small(description);
	}
	if ui.small_button("Open media").clicked() {
		*opening = resolve_media(&media.url, message).and_then(markdown::external_url);
	}
}

fn resolve_media<'a>(url: &'a str, message: &'a Message) -> Option<&'a str> {
	if let Some(filename) = url.strip_prefix("attachment://") {
		message
			.attachments
			.iter()
			.find(|attachment| attachment.filename == filename)
			.and_then(|attachment| attachment.media.url.as_deref())
	} else {
		Some(url)
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn modal_defaults_survive_nested_labels_without_replacing_explicit_values() {
		let mut components = vec![Component {
			kind: 18,
			component: Some(Box::new(Component {
				kind: 23,
				default: true,
				..Default::default()
			})),
			components: vec![Component {
				kind: 21,
				value: Some("chosen".into()),
				..Default::default()
			}],
			..Default::default()
		}];
		initialize(&mut components);
		assert_eq!(
			components[0].component.as_ref().unwrap().checked,
			Some(true)
		);
		assert_eq!(components[0].components[0].values, ["chosen"]);
		components[0].component.as_mut().unwrap().checked = Some(false);
		initialize(&mut components);
		assert_eq!(
			components[0].component.as_ref().unwrap().checked,
			Some(false)
		);
	}
}
