//! One bounded role draft backed by the existing server administration request lane.
use crate::{avatars::Avatars, design, dialog, emoji_picker::Picker, icons};
use client_core::{Command, State};
use egui::{Color32, RichText};
use model::{
	Id, Patch, permissions as p, server_admin,
	server_roles::{Action, Colors, Edit, Role},
};
use unicode_segmentation::UnicodeSegmentation;

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum Tab {
	#[default]
	Display,
	Permissions,
	Members,
}
#[derive(Clone, Copy)]
struct DragRole {
	generation: u64,
	guild: Id,
	role: Id,
}
#[derive(Default)]
pub(super) struct RolesUi {
	selected: Option<Id>,
	draft: Option<Role>,
	baseline: Option<Role>,
	revision: u64,
	search: String,
	permission_search: String,
	tab: Tab,
	submitted: bool,
	creating: bool,
	delete: Option<Id>,
	deleting: bool,
	switch_to: Option<Option<Id>>,
	error: Option<&'static str>,
	icon: Patch<String>,
	icon_texture: Option<egui::TextureHandle>,
	icon_pending: bool,
	icon_requested: bool,
	pub icon_request: u64,
	emoji_picker: Picker,
	member_query: server_admin::Query,
	member_search_changed: Option<f64>,
	members_requested: bool,
	adding_members: bool,
	preview_tab: Option<Tab>,
}
impl RolesUi {
	pub fn preview_editor(&mut self, permissions: bool) {
		self.preview_tab = Some(if permissions {
			Tab::Permissions
		} else {
			Tab::Display
		});
	}
	pub fn editing(&self) -> bool {
		self.selected.is_some()
	}
	pub fn has_changes(&self) -> bool {
		self.draft != self.baseline
			|| !matches!(self.icon, Patch::Absent)
			|| self.icon_pending
			|| self.submitted
	}
	pub fn select(&mut self, role: Option<Id>, guild: Id) {
		self.selected = role;
		self.draft = None;
		self.baseline = None;
		self.tab = if role == Some(guild) {
			Tab::Permissions
		} else {
			Tab::Display
		};
		self.icon = Patch::Absent;
		self.icon_texture = None;
		self.icon_pending = false;
		self.icon_requested = false;
		self.error = None;
		self.member_query = server_admin::Query::default();
		self.members_requested = false;
		self.adding_members = false;
	}
	fn switch(&mut self, role: Option<Id>, guild: Id) {
		if role == self.selected {
			return;
		}
		if self.has_changes() {
			self.switch_to = Some(role);
		} else {
			self.select(role, guild);
		}
	}
	pub fn load(&mut self, state: &mut State, guild: Id) -> Option<Command> {
		if state.server_admin.pending
			|| (state.server_admin.guild == Some(guild)
				&& (state.server_admin.roles.is_some() || state.server_admin.error.is_some()))
		{
			return None;
		}
		state.request_server_admin(guild, server_admin::Action::Roles(Action::Load))
	}
	pub fn take_icon_request(&mut self) -> Option<Id> {
		if !std::mem::take(&mut self.icon_requested) {
			return None;
		}
		self.selected
	}
	pub fn accept_icon(
		&mut self,
		ctx: &egui::Context,
		role: Id,
		request: u64,
		result: Result<Option<(String, egui::ColorImage)>, &'static str>,
	) {
		if self.selected != Some(role) || self.icon_request != request || !self.icon_pending {
			return;
		}
		self.icon_pending = false;
		match result {
			Ok(Some((data, image)))
				if data.len() <= server_admin::MAX_IMAGE_URI
					&& (1..=128).contains(&image.size[0])
					&& (1..=128).contains(&image.size[1])
					&& image.pixels.len() == image.size[0] * image.size[1] =>
			{
				self.icon = Patch::Value(data);
				self.icon_texture =
					Some(ctx.load_texture("role-icon-draft", image, egui::TextureOptions::LINEAR));
				if let Some(draft) = &mut self.draft {
					draft.unicode_emoji = None;
				}
				self.error = None;
			}
			Ok(None) => {}
			Ok(Some(_)) => self.error = Some("The prepared role icon exceeds its image limits."),
			Err(error) => self.error = Some(error),
		}
	}
	fn dispatch(state: &mut State, guild: Id, action: Action, commands: &mut Vec<Command>) -> bool {
		if let Some(command) =
			state.request_server_admin(guild, server_admin::Action::Roles(action))
		{
			commands.push(command);
			true
		} else {
			false
		}
	}
	fn sync(&mut self, state: &State, guild: Id) {
		if let Some(tab) = self.preview_tab
			&& let Some(role) = state.server_admin.roles.as_ref().and_then(|catalog| {
				catalog
					.items
					.iter()
					.find(|role| role.id != guild && state.can_edit_guild_role(guild, role.id))
			}) {
			self.select(Some(role.id), guild);
			self.tab = tab;
			self.preview_tab = None;
		}
		if self.creating && !state.server_admin.pending {
			self.creating = false;
			if state.server_admin.error.is_none() {
				self.select(state.server_admin.selected_role, guild);
			}
		}
		if self.deleting && !state.server_admin.pending {
			self.deleting = false;
			if state.server_admin.error.is_none() {
				self.delete = None;
			}
		}
		if self.submitted && !state.server_admin.pending {
			self.submitted = false;
			if state.server_admin.error.is_none() {
				self.draft = None;
				self.baseline = None;
				self.icon = Patch::Absent;
				self.icon_texture = None;
			}
		}
		let Some(role) = self.selected else {
			return;
		};
		let Some(catalog) = &state.server_admin.roles else {
			return;
		};
		let Some(fresh) = catalog.items.iter().find(|entry| entry.id == role) else {
			self.select(None, guild);
			return;
		};
		if !state.can_edit_guild_role(guild, role) {
			self.draft = Some(fresh.clone());
			self.baseline = Some(fresh.clone());
			self.icon = Patch::Absent;
			self.icon_texture = None;
			self.icon_pending = false;
			self.icon_requested = false;
		}
		if self.draft.is_none() || self.revision != state.server_admin.revision {
			let changes = self
				.baseline
				.as_ref()
				.zip(self.draft.as_ref())
				.map(|(before, after)| Edit::between(before, after));
			let mut draft = fresh.clone();
			if let Some(changes) = changes {
				changes.apply(&mut draft);
			}
			self.baseline = Some(fresh.clone());
			self.draft = Some(draft);
			self.revision = state.server_admin.revision;
		}
		if !state.can_edit_role_icon(guild, role) {
			self.icon = Patch::Absent;
			self.icon_texture = None;
			self.icon_pending = false;
			self.icon_requested = false;
			if let Some(draft) = &mut self.draft {
				draft.icon.clone_from(&fresh.icon);
				draft.unicode_emoji.clone_from(&fresh.unicode_emoji);
			}
		}
	}
	pub fn navigation(
		&mut self,
		ui: &mut egui::Ui,
		state: &mut State,
		guild: Id,
		commands: &mut Vec<Command>,
	) {
		ui.spacing_mut().item_spacing = egui::vec2(8.0, 5.0);
		let width = ui.available_width();
		ui.horizontal(|ui| {
			if ui
				.add_sized(
					[width - 36.0, 32.0],
					egui::Button::new("←  BACK").frame(false),
				)
				.clicked()
			{
				self.switch(None, guild);
			}
			if state.can_create_guild_role(guild)
				&& ui
					.add_enabled(
						!state.server_admin.pending
							&& !state.server_admin.needs_refresh
							&& !self.has_changes(),
						egui::Button::new("+"),
					)
					.on_hover_text("Create Role")
					.clicked()
			{
				self.creating = Self::dispatch(
					state,
					guild,
					Action::Create(Edit {
						name: Some("new role".into()),
						..Edit::default()
					}),
					commands,
				);
			}
		});
		ui.add_space(20.0);
		if let Some(catalog) = &state.server_admin.roles {
			egui::ScrollArea::vertical()
				.id_salt("role-navigation-scroll")
				.show(ui, |ui| {
					for role in &catalog.items {
						let color = rgb(role.colors.primary);
						let response = ui.add_sized(
							[width, 36.0],
							egui::Button::new(())
								.selected(self.selected == Some(role.id))
								.right_text(""),
						);
						ui.painter().circle_filled(
							response.rect.left_center() + egui::vec2(14.0, 0.0),
							6.0,
							color,
						);
						let name = egui::WidgetText::from(
							RichText::new(&role.name)
								.size(14.0)
								.color(design::palette(ui).text),
						)
						.into_galley(
							ui,
							Some(egui::TextWrapMode::Truncate),
							width - 34.0,
							egui::TextStyle::Body,
						);
						ui.painter().galley(
							response.rect.left_center() + egui::vec2(29.0, -name.size().y / 2.0),
							name,
							design::palette(ui).text,
						);
						response.widget_info(|| {
							egui::WidgetInfo::selected(
								egui::Role::Button,
								true,
								self.selected == Some(role.id),
								&role.name,
							)
						});
						if response.clicked() {
							self.switch(Some(role.id), guild);
						}
					}
				});
		}
	}
	pub fn show(
		&mut self,
		ui: &mut egui::Ui,
		state: &mut State,
		guild: Id,
		avatars: &mut Avatars,
		commands: &mut Vec<Command>,
	) {
		self.sync(state, guild);
		ui.spacing_mut().item_spacing = egui::vec2(8.0, 8.0);
		if let Some(error) = state.server_admin.error.or(self.error) {
			design::notice(ui, design::Level::Error, error);
			if ui
				.add_enabled(
					!state.server_admin.pending,
					egui::Button::new("Reload Roles"),
				)
				.clicked()
			{
				Self::dispatch(state, guild, Action::Load, commands);
				self.error = None;
				self.members_requested = false;
			}
		}
		if state.server_admin.pending {
			ui.horizontal(|ui| {
				ui.spinner();
				ui.weak(if state.server_admin.saving {
					"Saving..."
				} else {
					"Loading roles..."
				});
			});
		}
		if self.selected.is_some() {
			let width = ui.available_width().min(600.0);
			ui.allocate_ui_with_layout(
				egui::vec2(width, 0.0),
				egui::Layout::top_down(egui::Align::Min),
				|ui| {
					ui.set_width(width);
					self.editor(ui, state, guild, avatars, commands);
				},
			);
		} else {
			self.list(ui, state, guild, commands);
		}
		self.confirmations(ui.ctx(), state, guild, commands);
	}
	fn list(
		&mut self,
		ui: &mut egui::Ui,
		state: &mut State,
		guild: Id,
		commands: &mut Vec<Command>,
	) {
		let colors = design::palette(ui);
		ui.label(design::semibold(ui, "Roles", 22.0));
		ui.label("Use roles to group your server members and assign permissions.");
		ui.add_space(20.0);
		let width = ui.available_width();
		if ui
			.add_sized(
				[width, 76.0],
				egui::Button::new(
					RichText::new("Default Permissions\n@everyone · applies to all server members")
						.size(16.0),
				)
				.right_text("›")
				.fill(colors.raised)
				.corner_radius(10),
			)
			.clicked()
		{
			self.switch(Some(guild), guild);
		}
		ui.add_space(24.0);
		ui.horizontal(|ui| {
			ui.add(
				egui::TextEdit::singleline(&mut self.search)
					.hint_text("Search Roles")
					.char_limit(100)
					.desired_width((width - 128.0).max(60.0))
					.margin(egui::vec2(12.0, 10.0)),
			);
			if state.can_create_guild_role(guild)
				&& ui
					.add_enabled_ui(
						!state.server_admin.pending && !state.server_admin.needs_refresh,
						|ui| design::button(ui, "Create Role", design::ButtonKind::Primary),
					)
					.inner
					.clicked()
			{
				self.creating = Self::dispatch(
					state,
					guild,
					Action::Create(Edit {
						name: Some("new role".into()),
						..Edit::default()
					}),
					commands,
				);
			}
		});
		ui.label(
			"Members use the color of the highest role they have on this list. Drag roles to reorder them.",
		);
		ui.add_space(28.0);
		let Some(catalog) = &state.server_admin.roles else {
			return;
		};
		let query = self.search.to_lowercase();
		let count = catalog.items.iter().filter(|role| role.id != guild).count();
		ui.horizontal(|ui| {
			fixed_label(ui, &format!("ROLES — {count}"), width * 0.52, true);
			fixed_label(ui, "MEMBERS", (width * 0.48 - 108.0).max(48.0), true);
		});
		ui.separator();
		let mut action = None;
		for role in catalog.items.iter().filter(|role| {
			role.id != guild && (query.is_empty() || role.name.to_lowercase().contains(&query))
		}) {
			ui.push_id(role.id, |ui| {
				let row = ui
					.horizontal(|ui| {
						let color = if role.colors.primary == 0 {
							colors.muted
						} else {
							rgb(role.colors.primary)
						};
						let name_width = width * 0.52;
						let (rect, response) = ui.allocate_exact_size(
							egui::vec2(name_width, 58.0),
							egui::Sense::click_and_drag(),
						);
						let name = egui::WidgetText::from(
							RichText::new(&role.name).size(16.0).color(colors.text),
						)
						.into_galley(
							ui,
							Some(egui::TextWrapMode::Truncate),
							name_width - 32.0,
							egui::TextStyle::Body,
						);
						ui.painter().circle_filled(
							rect.left_center() + egui::vec2(9.0, 0.0),
							8.0,
							color,
						);
						ui.painter().galley(
							rect.left_center() + egui::vec2(28.0, -name.size().y / 2.0),
							name,
							colors.text,
						);
						response.widget_info(|| {
							egui::WidgetInfo::labeled(
								egui::Role::Button,
								true,
								format!("Edit role {}", role.name),
							)
						});
						if response.clicked() {
							self.switch(Some(role.id), guild);
						}
						if state.can_edit_guild_role(guild, role.id)
							&& !state.server_admin.pending
							&& self.search.is_empty()
						{
							response.dnd_set_drag_payload(DragRole {
								generation: state.generation,
								guild,
								role: role.id,
							});
						}
						fixed_label(
							ui,
							&role
								.member_count
								.map_or_else(|| "Unknown".into(), |count| count.to_string()),
							(width * 0.48 - 116.0).max(48.0),
							false,
						);
						if boxed_icon(ui, icons::Icon::Pencil, "Edit Role").clicked() {
							self.switch(Some(role.id), guild);
						}
						let menu = boxed_icon(ui, icons::Icon::More, "Role actions");
						egui::Popup::menu(&menu).show(|ui| {
							if ui.button("Edit Role").clicked() {
								self.switch(Some(role.id), guild);
								ui.close();
							}
							if state.can_move_guild_role(guild, role.id, role.position + 1)
								&& ui.button("Move Up").clicked()
							{
								action = Some(Action::Move {
									id: role.id,
									position: role.position + 1,
								});
								ui.close();
							}
							if state.can_move_guild_role(guild, role.id, role.position - 1)
								&& ui.button("Move Down").clicked()
							{
								action = Some(Action::Move {
									id: role.id,
									position: role.position - 1,
								});
								ui.close();
							}
							if state.can_delete_guild_role(guild, role.id)
								&& ui
									.button(RichText::new("Delete Role").color(colors.danger))
									.clicked()
							{
								self.delete = Some(role.id);
								ui.close();
							}
						});
					})
					.response;
				if let Some(source) = row.dnd_hover_payload::<DragRole>()
					&& source.generation == state.generation
					&& source.guild == guild
					&& source.role != role.id
					&& state.can_move_guild_role(guild, source.role, role.position)
				{
					ui.painter().hline(
						row.rect.x_range(),
						row.rect.top(),
						egui::Stroke::new(2.0, colors.accent),
					);
				}
				if let Some(source) = row.dnd_release_payload::<DragRole>()
					&& source.generation == state.generation
					&& source.guild == guild
					&& source.role != role.id
					&& self.search.is_empty()
					&& state.can_move_guild_role(guild, source.role, role.position)
				{
					action = Some(Action::Move {
						id: source.role,
						position: role.position,
					});
				}
				ui.separator();
			});
		}
		if let Some(action) = action
			&& !Self::dispatch(state, guild, action, commands)
		{
			self.error =
				Some("This role cannot be moved. Reload the roles and check your permissions.");
		}
	}
	fn editor(
		&mut self,
		ui: &mut egui::Ui,
		state: &mut State,
		guild: Id,
		avatars: &mut Avatars,
		commands: &mut Vec<Command>,
	) {
		ui.set_width(ui.available_width().min(600.0));
		if ui.ctx().content_rect().width() < 752.0 {
			let mut selected = self.selected;
			ui.horizontal_wrapped(|ui| {
				if ui.button("← Back to Roles").clicked() {
					selected = None;
				}
				egui::ComboBox::from_id_salt("compact-role-navigation")
					.selected_text("Choose role")
					.show_ui(ui, |ui| {
						if let Some(catalog) = &state.server_admin.roles {
							for role in &catalog.items {
								ui.selectable_value(&mut selected, Some(role.id), &role.name);
							}
						}
					});
			});
			if selected != self.selected {
				self.switch(selected, guild);
			}
		}
		let Some(draft) = &self.draft else {
			return;
		};
		let role = draft.id;
		let colors = design::palette(ui);
		let width = ui.available_width();
		ui.horizontal(|ui| {
			fixed_label(
				ui,
				&format!("EDIT ROLE — {}", draft.name),
				(width - 40.0).max(80.0),
				true,
			);
			if state.can_delete_guild_role(guild, role) {
				let button = icons::button(ui, icons::Icon::More, 28.0, "Role actions");
				egui::Popup::menu(&button).show(|ui| {
					if ui
						.button(RichText::new("Delete Role").color(colors.danger))
						.clicked()
					{
						self.delete = Some(role);
						ui.close();
					}
				});
			}
		});
		ui.add_space(22.0);
		ui.horizontal_wrapped(|ui| {
			if role != guild {
				tab_button(ui, &mut self.tab, Tab::Display, "Display");
			}
			tab_button(ui, &mut self.tab, Tab::Permissions, "Permissions");
			if role != guild && state.can_open_member_settings(guild) {
				tab_button(
					ui,
					&mut self.tab,
					Tab::Members,
					&format!(
						"Manage Members ({})",
						draft
							.member_count
							.map_or_else(|| "?".into(), |count| count.to_string())
					),
				);
			}
		});
		ui.separator();
		ui.add_space(20.0);
		if !state.can_edit_guild_role(guild, role) {
			ui.weak(if draft.managed {
				"This role is managed by an integration."
			} else {
				"This role is above your highest role and is read-only."
			});
		}
		if self.tab == Tab::Members {
			if state.can_open_member_settings(guild) {
				self.members(ui, state, guild, role, avatars, commands);
			} else {
				self.tab = Tab::Display;
			}
			return;
		}
		let editable = state.can_edit_guild_role(guild, role)
			&& !state.server_admin.pending
			&& !state.server_admin.needs_refresh;
		if self.tab == Tab::Display {
			ui.add_enabled_ui(editable, |ui| self.display(ui, state, guild, avatars));
		} else if let Some(draft) = &mut self.draft {
			permissions(
				ui,
				state,
				guild,
				draft,
				&mut self.permission_search,
				editable,
			);
		}
	}
	fn display(&mut self, ui: &mut egui::Ui, state: &State, guild: Id, avatars: &mut Avatars) {
		let Some(draft) = &mut self.draft else {
			return;
		};
		let colors = design::palette(ui);
		let label = design::label(ui, "Role name");
		design::input(
			ui,
			egui::TextEdit::singleline(&mut draft.name).char_limit(100),
		)
		.labelled_by(label.id);
		section(ui, "Role Style");
		let enhanced = state.can_use_enhanced_role_colors(guild);
		let style = if draft.colors.tertiary.is_some() {
			2
		} else if draft.colors.secondary.is_some() {
			1
		} else {
			0
		};
		let card_width = ((ui.available_width() - 16.0)
			/ if enhanced || style != 0 { 3.0 } else { 1.0 })
		.min(180.0);
		ui.horizontal(|ui| {
			for (choice, name) in [(0, "Solid"), (1, "Gradient"), (2, "Holographic")] {
				if choice != 0 && !enhanced && choice != style {
					continue;
				}
				let sample = match choice {
					1 => Colors {
						primary: if draft.colors.primary == 0 {
							design::DEFAULT_PRIMARY_RGB
						} else {
							draft.colors.primary
						},
						secondary: Some(0x9b59b6),
						tertiary: None,
					},
					2 => Colors {
						primary: 11127295,
						secondary: Some(16759788),
						tertiary: Some(16761760),
					},
					_ => Colors {
						primary: draft.colors.primary,
						secondary: None,
						tertiary: None,
					},
				};
				let response = ui
					.add_enabled_ui(choice == 0 || enhanced, |ui| {
						let frame = egui::Frame::new()
							.fill(colors.raised)
							.stroke(egui::Stroke::new(
								if style == choice { 2.0 } else { 1.0 },
								if style == choice {
									colors.accent
								} else {
									colors.border
								},
							))
							.corner_radius(8)
							.inner_margin(12)
							.show(ui, |ui| {
								ui.vertical(|ui| {
									ui.set_width((card_width - 24.0).max(30.0));
									ui.set_min_height(66.0);
									ui.horizontal(|ui| {
										let (rect, _) = ui.allocate_exact_size(
											egui::vec2(12.0, 12.0),
											egui::Sense::hover(),
										);
										ui.painter().circle_filled(
											rect.center(),
											5.0,
											rgb(sample.primary),
										);
										ui.add(
											egui::Label::new(design::semibold(ui, "Preview", 14.0))
												.truncate(),
										);
									});
									ui.add(
										egui::Label::new(RichText::new("Sample message").small())
											.truncate(),
									);
									ui.add_space(8.0);
									ui.add(egui::Label::new(name).truncate());
								});
							});
						ui.interact(
							frame.response.rect,
							ui.scope_id().with(("role-style", choice)),
							egui::Sense::click(),
						)
					})
					.inner;
				response.widget_info(|| {
					egui::WidgetInfo::selected(
						egui::Role::RadioButton,
						choice == 0 || enhanced,
						style == choice,
						name,
					)
				});
				if response.clicked() {
					draft.colors = match choice {
						2 => Colors {
							primary: 11127295,
							secondary: Some(16759788),
							tertiary: Some(16761760),
						},
						1 => Colors {
							primary: if draft.colors.primary == 0 {
								design::DEFAULT_PRIMARY_RGB
							} else {
								draft.colors.primary
							},
							secondary: Some(0x9b59b6),
							tertiary: None,
						},
						_ => Colors {
							primary: draft.colors.primary,
							secondary: None,
							tertiary: None,
						},
					};
				}
			}
		});
		section(ui, "Role color");
		ui.weak("Members use the color of their highest role on the roles list.");
		let palette = [
			0x1abc9c, 0x2ecc71, 0x3498db, 0x9b59b6, 0xe91e63, 0xf1c40f, 0xe67e22, 0xe74c3c,
			0x95a5a6, 0x607d8b, 0x11806a, 0x1f8b4c, 0x206694, 0x71368a, 0xad1457, 0xc27c0e,
			0xa84300, 0x992d22, 0x979c9f, 0x546e7a,
		];
		let palette_columns = (((ui.available_width() - 208.0) / 32.0) as usize).clamp(1, 10);
		ui.add_enabled_ui(
			style != 2 && (enhanced || draft.colors.secondary.is_none()),
			|ui| {
				ui.horizontal_top(|ui| {
					let (rect, response) =
						ui.allocate_exact_size(egui::vec2(48.0, 48.0), egui::Sense::click());
					ui.painter().rect_filled(rect, 8.0, colors.muted);
					if draft.colors.primary == 0 {
						icons::paint(
							ui.painter(),
							icons::Icon::Check,
							rect.shrink(15.0),
							colors.text,
						);
					}
					if response.on_hover_text("Default role color").clicked() {
						draft.colors.primary = 0;
					}
					let mut color = [
						(draft.colors.primary >> 16) as u8,
						(draft.colors.primary >> 8) as u8,
						draft.colors.primary as u8,
					];
					if design::color_edit(ui, &mut color)
						.on_hover_text("Custom role color")
						.changed()
					{
						draft.colors.primary = (u32::from(color[0]) << 16)
							| (u32::from(color[1]) << 8)
							| u32::from(color[2]);
					}
					ui.vertical(|ui| {
						for row in palette.chunks(palette_columns) {
							ui.horizontal(|ui| {
								for &color in row {
									let (rect, response) = ui.allocate_exact_size(
										egui::vec2(24.0, 24.0),
										egui::Sense::click(),
									);
									ui.painter().circle_filled(rect.center(), 12.0, rgb(color));
									if draft.colors.primary == color {
										icons::paint(
											ui.painter(),
											icons::Icon::Check,
											rect.shrink(5.0),
											Color32::WHITE,
										);
									}
									if response.on_hover_text(format!("#{color:06X}")).clicked() {
										draft.colors.primary = color;
									}
								}
							});
						}
					});
				});
			},
		);
		if enhanced
			&& draft.colors.tertiary.is_none()
			&& let Some(secondary) = &mut draft.colors.secondary
		{
			ui.horizontal(|ui| {
				ui.label("Second gradient color");
				let mut color = [
					(*secondary >> 16) as u8,
					(*secondary >> 8) as u8,
					*secondary as u8,
				];
				if design::color_edit(ui, &mut color).changed() {
					*secondary = (u32::from(color[0]) << 16)
						| (u32::from(color[1]) << 8)
						| u32::from(color[2]);
				}
			});
		}
		if state.can_edit_role_icon(guild, draft.id) {
			section(ui, "Role icon");
			ui.weak(
				"Upload an image under 256 KiB or choose a Unicode emoji. We recommend at least 64×64 pixels.",
			);
			ui.horizontal(|ui| {
				if ui
					.add_enabled(
						!self.icon_pending,
						egui::Button::new(if self.icon_pending {
							"Preparing..."
						} else {
							"Choose Image"
						})
						.min_size(egui::vec2(120.0, 36.0)),
					)
					.clicked()
				{
					self.icon_pending = true;
					self.icon_requested = true;
				}
				let before = draft.unicode_emoji.clone();
				self.emoji_picker
					.unicode_button(ui, &mut draft.unicode_emoji);
				if before != draft.unicode_emoji {
					self.icon = Patch::Null;
					self.icon_texture = None;
					self.icon_pending = false;
					self.icon_requested = false;
					draft.icon = None;
				}
				if (draft.icon.is_some()
					|| draft.unicode_emoji.is_some()
					|| !matches!(self.icon, Patch::Absent))
					&& ui.button("Remove Icon").clicked()
				{
					draft.icon = None;
					draft.unicode_emoji = None;
					self.icon = Patch::Null;
					self.icon_texture = None;
					self.icon_pending = false;
					self.icon_requested = false;
				}
			});
		}
		ui.add_space(20.0);
		for background in [colors.raised, colors.sidebar, colors.chat] {
			egui::Frame::new()
				.fill(background)
				.inner_margin(16)
				.show(ui, |ui| {
					ui.set_width((ui.available_width() - 32.0).max(100.0));
					ui.horizontal_top(|ui| {
						if let Some(user) = &state.user {
							avatars.show(ui, user, 40.0, state.demo);
						}
						ui.vertical(|ui| {
							ui.horizontal(|ui| {
								colored_name(
									ui,
									state
										.user
										.as_ref()
										.map_or("Preview", |user| user.name.as_str()),
									draft.colors,
									14.0,
								);
								if let Some(texture) = &self.icon_texture {
									ui.add(
										egui::Image::from_texture(texture)
											.fit_to_exact_size(egui::Vec2::splat(18.0)),
									);
								} else if let Some(emoji) = &draft.unicode_emoji {
									if let Some(image) = crate::emoji::image(ui.ctx(), emoji, 18.0)
									{
										ui.add(image);
									}
								} else if let Some(hash) = draft
									.icon
									.as_ref()
									.filter(|_| !matches!(self.icon, Patch::Null))
								{
									avatars.show_icon(
										ui,
										Some(format!("role-icon-{}-{hash}", draft.id)),
										18.0,
										state.demo,
										"Role icon",
									);
								}
							});
							ui.label("This is how members with this role appear.");
						});
					});
				});
		}
		ui.add_space(20.0);
		design::switch(
			ui,
			"Display role members separately from online members",
			None,
			&mut draft.hoist,
		);
		design::switch(
			ui,
			"Allow anyone to @mention this role",
			Some("Members with permission to mention all roles can always mention this role."),
			&mut draft.mentionable,
		);
	}
	pub fn save_bar(
		&mut self,
		ui: &mut egui::Ui,
		state: &mut State,
		guild: Id,
		commands: &mut Vec<Command>,
	) {
		let (save, reset) = design::save_bar(
			ui,
			self.submitted.then_some("Saving role…"),
			!state.server_admin.pending && !state.server_admin.needs_refresh && !self.icon_pending,
			!state.server_admin.pending,
		);
		if reset {
			self.draft.clone_from(&self.baseline);
			self.icon = Patch::Absent;
			self.icon_texture = None;
			self.icon_pending = false;
			self.icon_requested = false;
			self.error = None;
		}
		if save && let (Some(before), Some(draft)) = (&self.baseline, &self.draft) {
			let mut edit = Edit::between(before, draft);
			if !matches!(self.icon, Patch::Absent) {
				edit.icon = self.icon.clone();
			}
			self.submitted =
				Self::dispatch(state, guild, Action::Edit { id: draft.id, edit }, commands);
			if !self.submitted {
				self.error = Some(
					"Could not save this role. Check the name, permissions, and your role hierarchy.",
				);
			}
		}
	}
	fn members(
		&mut self,
		ui: &mut egui::Ui,
		state: &mut State,
		guild: Id,
		role: Id,
		avatars: &mut Avatars,
		commands: &mut Vec<Command>,
	) {
		let filter = (!self.adding_members).then_some(role);
		let mut load = (!self.members_requested || state.server_admin.member_role_filter != filter)
			&& state.server_admin.error.is_none()
			&& self.error.is_none();
		ui.horizontal_wrapped(|ui| {
			if ui
				.add(
					egui::TextEdit::singleline(&mut self.member_query.search)
						.hint_text("Search members")
						.char_limit(100)
						.desired_width(220.0),
				)
				.changed()
			{
				self.member_query.after = None;
				self.member_search_changed = Some(ui.input(|input| input.time));
			}
			if ui
				.add_enabled(
					!state.server_admin.pending,
					egui::Button::new(if self.adding_members {
						"Back to Role Members"
					} else {
						"Add Members"
					}),
				)
				.clicked()
			{
				self.adding_members = !self.adding_members;
				self.member_query.after = None;
				load = true;
			}
		});
		if !state.server_admin.pending
			&& self
				.member_search_changed
				.is_some_and(|at| ui.input(|input| input.time) - at > 0.3)
		{
			self.member_search_changed = None;
			load = true;
		}
		if self.member_search_changed.is_some() {
			ui.ctx()
				.request_repaint_after(std::time::Duration::from_millis(300));
		}
		if load && !state.server_admin.pending {
			self.members_requested = true;
			if Self::dispatch(
				state,
				guild,
				Action::Members {
					role: (!self.adding_members).then_some(role),
					query: self.member_query.clone(),
				},
				commands,
			) {
				self.error = None;
			} else {
				self.error = Some(
					"Role members could not be loaded. Reload or change the search to try again.",
				);
			}
		}
		let mut action = None;
		if !state.server_admin.pending
			&& let Some(members) = &state.server_admin.members
		{
			for member in &members.items {
				if !self.adding_members && !member.roles.contains(&role) {
					continue;
				}
				let assigned = member.roles.contains(&role);
				let width = ui.available_width();
				ui.horizontal(|ui| {
					avatars.show(ui, &member.user, 32.0, state.demo);
					fixed_label(
						ui,
						member.nick.as_deref().unwrap_or(&member.user.name),
						(width - 140.0).max(70.0),
						false,
					);
					if state.can_edit_member_role(guild, member.user.id, role)
						&& ui
							.add_enabled(
								!state.server_admin.pending && !state.server_admin.needs_refresh,
								egui::Button::new(if assigned { "Remove" } else { "Add" }),
							)
							.clicked()
					{
						action = Some(server_admin::Action::SetRole {
							user: member.user.id,
							role,
							assigned: !assigned,
						});
					}
				});
				ui.separator();
			}
			ui.horizontal(|ui| {
				ui.weak(format!("Showing {} members", members.items.len()));
				if self.member_query.after.is_some()
					&& ui
						.add_enabled(!state.server_admin.pending, egui::Button::new("First page"))
						.clicked()
				{
					self.member_query.after = None;
					self.members_requested = false;
				}
				if let Some(next) = members.next
					&& ui
						.add_enabled(!state.server_admin.pending, egui::Button::new("Next page"))
						.clicked()
				{
					self.member_query.after = Some(next);
					self.members_requested = false;
				}
			});
		}
		if let Some(action) = action
			&& let Some(command) = state.request_server_admin(guild, action)
		{
			commands.push(command);
		}
	}
	fn confirmations(
		&mut self,
		ctx: &egui::Context,
		state: &mut State,
		guild: Id,
		commands: &mut Vec<Command>,
	) {
		if let Some(target) = self.switch_to {
			match dialog::Confirm::new(
				"discard-role-draft",
				"Discard role changes?",
				"Your unsaved changes to this role will be lost.",
			)
			.danger()
			.confirm_label("Discard Changes")
			.cancel_label("Keep Editing")
			.enabled(!state.server_admin.saving)
			.show(ctx)
			{
				Some(dialog::Choice::Confirmed) => {
					self.switch_to = None;
					self.select(target, guild);
				}
				Some(dialog::Choice::Cancelled) => self.switch_to = None,
				None => {}
			}
		}
		if let Some(role) = self.delete {
			let name = state
				.server_admin
				.roles
				.as_ref()
				.and_then(|catalog| catalog.items.iter().find(|known| known.id == role))
				.map_or("this role", |role| role.name.as_str());
			let mut confirm = dialog::Confirm::new(
				"delete-server-role",
				"Delete role?",
				format!("Members will lose every permission {name} grants. This cannot be undone."),
			)
			.danger()
			.confirm_label("Delete Role")
			.enabled(
				!state.server_admin.pending
					&& !state.server_admin.needs_refresh
					&& state.can_delete_guild_role(guild, role),
			);
			if let Some(error) = state.server_admin.error {
				confirm = confirm.note(dialog::Level::Error, error);
			}
			let choice = confirm.show(ctx);
			if choice == Some(dialog::Choice::Cancelled) && !state.server_admin.saving {
				self.delete = None;
			}
			if choice == Some(dialog::Choice::Confirmed) {
				self.deleting = Self::dispatch(state, guild, Action::Delete(role), commands);
			}
		}
	}
}
fn rgb(color: u32) -> Color32 {
	if color == 0 {
		Color32::from_rgb(153, 170, 181)
	} else {
		Color32::from_rgb((color >> 16) as u8, (color >> 8) as u8, color as u8)
	}
}
fn tab_button(ui: &mut egui::Ui, tab: &mut Tab, value: Tab, label: &str) {
	let colors = design::palette(ui);
	let response = ui.add(
		egui::Button::new(design::medium(ui, label, 15.0).color(if *tab == value {
			colors.text
		} else {
			colors.muted
		}))
		.frame(false)
		.min_size(egui::vec2(80.0, 32.0)),
	);
	if response.clicked() {
		*tab = value;
	}
	if *tab == value {
		ui.painter().hline(
			response.rect.x_range(),
			response.rect.bottom(),
			egui::Stroke::new(2.0, colors.accent),
		);
	}
	response.widget_info(|| {
		egui::WidgetInfo::selected(
			egui::Role::RadioButton,
			ui.is_enabled(),
			*tab == value,
			label,
		)
	});
}
fn boxed_icon(ui: &mut egui::Ui, icon: icons::Icon, label: &str) -> egui::Response {
	let colors = design::palette(ui);
	let response = ui.add_sized(
		[36.0, 36.0],
		egui::Button::new(()).fill(colors.raised).corner_radius(8),
	);
	icons::paint(ui.painter(), icon, response.rect.shrink(9.0), colors.text);
	response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), label));
	response.on_hover_text(label)
}
fn colored_name(ui: &mut egui::Ui, text: &str, colors: Colors, size: f32) {
	let palette = design::palette(ui);
	let readable =
		|value| design::role_name_color(value, ui.visuals().extreme_bg_color, palette.text);
	let primary = readable(colors.primary);
	let secondary = colors.secondary.map_or(primary, readable);
	let tertiary = colors.tertiary.map_or(secondary, readable);
	let count = text.graphemes(true).count().max(2) - 1;
	let mut job = egui::text::LayoutJob::default();
	for (index, grapheme) in text.graphemes(true).enumerate() {
		let at = index as f32 / count as f32;
		let (from, to, amount) = if colors.tertiary.is_some() {
			if at < 0.5 {
				(primary, secondary, at * 2.0)
			} else {
				(secondary, tertiary, (at - 0.5) * 2.0)
			}
		} else {
			(primary, secondary, at)
		};
		let channel =
			|a: u8, b: u8| (f32::from(a) + (f32::from(b) - f32::from(a)) * amount).round() as u8;
		job.append(
			grapheme,
			0.0,
			egui::TextFormat {
				font_id: egui::FontId::new(size, design::semibold_family(ui.ctx())),
				color: Color32::from_rgb(
					channel(from.r(), to.r()),
					channel(from.g(), to.g()),
					channel(from.b(), to.b()),
				),
				..Default::default()
			},
		);
	}
	ui.add(egui::Label::new(job).truncate());
}
fn section(ui: &mut egui::Ui, title: &str) {
	design::divider(ui);
	design::section(ui, title, None);
}
fn fixed_label(ui: &mut egui::Ui, text: &str, width: f32, strong: bool) {
	ui.allocate_ui_with_layout(
		egui::vec2(width, 36.0),
		egui::Layout::left_to_right(egui::Align::Center),
		|ui| {
			ui.set_width(width);
			ui.add(
				egui::Label::new(if strong {
					design::semibold(ui, text, 14.0)
				} else {
					RichText::new(text).size(14.0)
				})
				.truncate(),
			)
			.on_hover_text(text);
		},
	);
}
fn permissions(
	ui: &mut egui::Ui,
	state: &State,
	guild: Id,
	role: &mut Role,
	search: &mut String,
	editable: bool,
) {
	ui.add(
		egui::TextEdit::singleline(search)
			.char_limit(64)
			.hint_text("Search permissions")
			.desired_width(f32::INFINITY),
	);
	let query = search.to_lowercase();
	for (group, values) in [
		(
			"General Server Permissions",
			&[
				(p::VIEW_CHANNEL, "View Channels"),
				(p::MANAGE_CHANNELS, "Manage Channels"),
				(p::MANAGE_ROLES, "Manage Roles"),
				(p::MANAGE_GUILD, "Manage Server"),
				(p::CREATE_GUILD_EXPRESSIONS, "Create Expressions"),
				(p::MANAGE_GUILD_EXPRESSIONS, "Manage Expressions"),
			][..],
		),
		(
			"Membership Permissions",
			&[
				(p::CHANGE_NICKNAME, "Change Nickname"),
				(p::MANAGE_NICKNAMES, "Manage Nicknames"),
				(p::KICK_MEMBERS, "Kick Members"),
				(p::MODERATE_MEMBERS, "Timeout Members"),
			][..],
		),
		(
			"Text Channel Permissions",
			&[
				(p::SEND_MESSAGES, "Send Messages"),
				(p::SEND_MESSAGES_IN_THREADS, "Send Messages in Threads"),
				(p::CREATE_PUBLIC_THREADS, "Create Public Threads"),
				(p::CREATE_PRIVATE_THREADS, "Create Private Threads"),
				(p::EMBED_LINKS, "Embed Links"),
				(p::ATTACH_FILES, "Attach Files"),
				(p::ADD_REACTIONS, "Add Reactions"),
				(p::USE_EXTERNAL_EMOJIS, "Use External Emoji"),
				(p::USE_EXTERNAL_STICKERS, "Use External Stickers"),
				(
					p::MENTION_EVERYONE,
					"Mention @everyone, @here, and All Roles",
				),
				(p::MANAGE_MESSAGES, "Manage Messages"),
				(p::PIN_MESSAGES, "Pin Messages"),
				(p::MANAGE_THREADS, "Manage Threads"),
				(p::READ_MESSAGE_HISTORY, "Read Message History"),
				(p::SEND_TTS_MESSAGES, "Send Text-to-Speech Messages"),
			][..],
		),
		(
			"Voice Channel Permissions",
			&[
				(p::CONNECT, "Connect"),
				(p::SPEAK, "Speak"),
				(p::STREAM, "Video"),
				(p::USE_VAD, "Use Voice Activity"),
				(p::MUTE_MEMBERS, "Mute Members"),
				(p::DEAFEN_MEMBERS, "Deafen Members"),
				(p::MOVE_MEMBERS, "Move Members"),
			][..],
		),
		(
			"Advanced Permissions",
			&[(p::ADMINISTRATOR, "Administrator")][..],
		),
	] {
		if !values
			.iter()
			.any(|(_, label)| label.to_lowercase().contains(&query))
		{
			continue;
		}
		section(ui, group);
		for &(bit, label) in values
			.iter()
			.filter(|(_, label)| label.to_lowercase().contains(&query))
		{
			let mut enabled = role.permissions & bit != 0;
			ui.add_enabled_ui(editable && (enabled || state.can_grant_role_permission(guild, bit)), |ui| {
				let detail = (bit == p::ADMINISTRATOR).then_some("Grants every permission and bypasses channel permission overrides. Only grant this to people you trust.");
				if design::switch(ui, label, detail, &mut enabled).changed() { if enabled { role.permissions |= bit; } else { role.permissions &= !bit; } }
			});
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	fn fixture() -> State {
		let mut state = test_support::demo_state();
		state.auth = client_core::auth::AuthState::Authenticated;
		state.gateway_connected = true;
		let guild = state.guilds[0].id;
		let roles = vec![
			Role {
				id: Id(900),
				name: "Manager".into(),
				position: 3,
				permissions: p::MANAGE_ROLES | p::VIEW_CHANNEL,
				..Default::default()
			},
			Role {
				id: Id(901),
				name: "Member".into(),
				position: 1,
				permissions: 1 << 110,
				..Default::default()
			},
			Role {
				id: guild,
				name: "@everyone".into(),
				..Default::default()
			},
		];
		state.permissions.guilds.insert(
			guild,
			p::Guild {
				id: guild,
				owner: Some(Id(9999)),
				roles: Some(roles.iter().map(Role::permission_role).collect()),
				member: Some(p::Member {
					roles: vec![Id(900)],
					timeout_until: None,
				}),
			},
		);
		state.permissions.clear_cache();
		state.server_admin.guild = Some(guild);
		state.server_admin.roles = Some(model::server_roles::Catalog {
			guild,
			items: roles,
			features: vec![],
		});
		state
	}
	fn find_text(shape: &egui::Shape, target: &str) -> Option<egui::Pos2> {
		match shape {
			egui::Shape::Text(text) if text.galley.job.text == target => {
				Some(text.pos + text.galley.rect.size() / 2.0)
			}
			egui::Shape::Vec(shapes) => shapes.iter().find_map(|shape| find_text(shape, target)),
			_ => None,
		}
	}
	fn frame(
		ctx: &egui::Context,
		editor: &mut RolesUi,
		state: &mut State,
		commands: &mut Vec<Command>,
		events: Vec<egui::Event>,
		save: bool,
		target: &str,
	) -> Option<egui::Pos2> {
		let guild = state.guilds[0].id;
		let output = ctx.run_ui(
			egui::RawInput {
				screen_rect: Some(egui::Rect::from_min_size(
					egui::Pos2::ZERO,
					egui::vec2(850.0, 1600.0),
				)),
				events,
				..Default::default()
			},
			|ui| {
				ui.set_width(800.0);
				if save {
					editor.save_bar(ui, state, guild, commands);
				} else {
					editor.show(ui, state, guild, &mut Avatars::default(), commands);
				}
			},
		);
		let point = output
			.shapes
			.iter()
			.find_map(|shape| find_text(&shape.shape, target));
		output.drop_without_applying_deltas();
		point
	}
	fn click(point: egui::Pos2) -> Vec<egui::Event> {
		vec![
			egui::Event::PointerMoved(point),
			egui::Event::PointerButton {
				pos: point,
				button: egui::PointerButton::Primary,
				pressed: true,
				modifiers: Default::default(),
			},
			egui::Event::PointerButton {
				pos: point,
				button: egui::PointerButton::Primary,
				pressed: false,
				modifiers: Default::default(),
			},
		]
	}
	#[test]
	fn role_list_selection_saves_partial_edit_and_revocation_discards_draft() {
		let ctx = egui::Context::default();
		design::apply(&ctx);
		let mut state = fixture();
		let guild = state.guilds[0].id;
		let mut editor = RolesUi::default();
		let mut commands = vec![];
		let member = frame(
			&ctx,
			&mut editor,
			&mut state,
			&mut commands,
			vec![],
			false,
			"Member",
		)
		.unwrap();
		frame(
			&ctx,
			&mut editor,
			&mut state,
			&mut commands,
			click(member),
			false,
			"",
		);
		assert_eq!(editor.selected, Some(Id(901)));
		frame(
			&ctx,
			&mut editor,
			&mut state,
			&mut commands,
			vec![],
			false,
			"",
		);
		editor.draft.as_mut().unwrap().name = "Renamed".into();
		let save = frame(
			&ctx,
			&mut editor,
			&mut state,
			&mut commands,
			vec![],
			true,
			"Save Changes",
		)
		.unwrap();
		frame(
			&ctx,
			&mut editor,
			&mut state,
			&mut commands,
			click(save),
			true,
			"",
		);
		let Command::ServerAdmin { action, .. } = commands.pop().unwrap() else {
			panic!("expected role edit");
		};
		let server_admin::Action::Roles(Action::Edit { id, edit }) = *action else {
			panic!("expected role edit");
		};
		assert_eq!(id, Id(901));
		assert_eq!(edit.name.as_deref(), Some("Renamed"));
		assert_eq!(edit.permissions, None);
		assert_eq!(edit.permission_mask, 0);
		assert!(matches!(edit.icon, Patch::Absent));
		state.server_admin.pending = false;
		state.server_admin.error = Some("synthetic failed save");
		state.permissions.guilds.get_mut(&guild).unwrap().member = None;
		state.permissions.clear_cache();
		editor.sync(&state, guild);
		assert!(!editor.has_changes());
		assert_eq!(editor.draft.as_ref().unwrap().name, "Member");
		assert_eq!(editor.draft.as_ref().unwrap().permissions, 1 << 110);
	}
}
