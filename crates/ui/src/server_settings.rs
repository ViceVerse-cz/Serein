//! One permission-checked, session-only server settings draft.
use crate::{MessagingUi, avatars::Avatars, design, dialog, settings::close_control};
use client_core::{Command, State};
use egui::Color32;
use model::{
	Id, Patch,
	server_settings::{Edit, Settings, Trait},
};

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum Page {
	#[default]
	Profile,
	Engagement,
	Emoji,
	Members,
	Roles,
	Invites,
	Integrations,
	AuditLog,
}
impl Page {
	fn allowed(self, state: &State, guild: Id) -> bool {
		match self {
			Self::Profile | Self::Engagement => state.can_manage_guild(guild),
			Self::Emoji => state.can_open_emoji_settings(guild),
			Self::Members => state.can_open_member_settings(guild),
			Self::Roles => state.can_open_role_settings(guild),
			Self::Invites => state.can_open_invite_settings(guild),
			Self::Integrations => state.can_open_integration_settings(guild),
			Self::AuditLog => state.can_open_audit_log_settings(guild),
		}
	}
	fn label(self) -> &'static str {
		match self {
			Self::Profile => "Server Profile",
			Self::Engagement => "Engagement",
			Self::Emoji => "Emoji",
			Self::Members => "Members",
			Self::Roles => "Roles",
			Self::Invites => "Invites",
			Self::Integrations => "Integrations",
			Self::AuditLog => "Audit Log",
		}
	}
}

#[derive(Default)]
pub(super) struct Editor {
	scope: Option<(u64, Id)>,
	page: Page,
	draft: Option<Settings>,
	baseline: Option<Settings>,
	revision: u64,
	submitted: bool,
	discard: bool,
	icon: Patch<String>,
	icon_preview: Option<egui::TextureHandle>,
	icon_request: u64,
	icon_requested: bool,
	icon_pending: bool,
	icon_error: Option<&'static str>,
	form_error: Option<&'static str>,
	emoji_picker: crate::emoji_picker::Picker,
	pub(super) admin: crate::server_admin::Admin,
	roles: crate::server_roles::RolesUi,
	invites: crate::server_invites::InvitesUi,
	integrations: crate::server_integrations::IntegrationsUi,
	audit_log: crate::server_audit_log::AuditLogUi,
}

impl MessagingUi {
	/// Select the engagement page in the offline preview harness.
	pub fn preview_server_engagement(&mut self) {
		self.server_settings.page = Page::Engagement;
	}
	pub fn preview_server_roles(
		&mut self,
		state: &mut State,
		guild: Id,
		role: Option<Id>,
	) -> Option<Command> {
		if !state.can_open_role_settings(guild) {
			return None;
		}
		self.settings.open = false;
		self.server_settings = Editor {
			scope: Some((state.generation, guild)),
			page: Page::Roles,
			..Editor::default()
		};
		self.server_settings.roles.select(role, guild);
		self.server_settings.roles.load(state, guild)
	}
	pub fn take_server_role_icon_request(&mut self) -> Option<(u64, Id, Id, u64)> {
		let (generation, guild) = self.server_settings.scope?;
		let role = self.server_settings.roles.take_icon_request()?;
		self.server_role_icon_sequence = self.server_role_icon_sequence.wrapping_add(1);
		self.server_settings.roles.icon_request = self.server_role_icon_sequence;
		Some((generation, guild, role, self.server_role_icon_sequence))
	}
	pub fn accept_server_role_icon(
		&mut self,
		ctx: &egui::Context,
		scope: (u64, Id, Id, u64),
		result: Result<Option<(String, egui::ColorImage)>, &'static str>,
	) {
		if self.server_settings.scope == Some((scope.0, scope.1)) {
			self.server_settings
				.roles
				.accept_icon(ctx, scope.2, scope.3, result);
		}
	}
	pub fn preview_server_admin(
		&mut self,
		state: &mut State,
		guild: Id,
		page: &str,
	) -> Option<Command> {
		if matches!(page, "roles" | "role-editor" | "role-permissions") {
			let command = self.preview_server_roles(state, guild, None);
			if page != "roles" {
				self.server_settings
					.roles
					.preview_editor(page == "role-permissions");
			}
			return command;
		}
		let webhooks = page == "webhooks";
		let expand_audit = page == "audit-log-expanded";
		let page = if matches!(page, "audit-log" | "audit-log-expanded") {
			Page::AuditLog
		} else if matches!(page, "integrations" | "webhooks") {
			Page::Integrations
		} else if page == "invites" {
			Page::Invites
		} else if page == "members" {
			Page::Members
		} else {
			Page::Emoji
		};
		if !page.allowed(state, guild) {
			return None;
		}
		self.settings.open = false;
		self.server_settings = Editor {
			scope: Some((state.generation, guild)),
			page,
			..Editor::default()
		};
		if page == Page::AuditLog {
			self.server_settings.audit_log.preview(expand_audit);
			self.server_settings.audit_log.load(state, guild)
		} else if page == Page::Integrations {
			self.server_settings.integrations.preview(webhooks);
			self.server_settings.integrations.load(state, guild)
		} else if page == Page::Invites {
			self.server_settings.invites.load(state, guild)
		} else {
			self.server_settings
				.admin
				.load(state, guild, page == Page::Members)
		}
	}
	pub fn accepts_server_emoji_drops(&self) -> bool {
		self.server_settings.is_open() && self.server_settings.page == Page::Emoji
	}
	pub fn queue_server_emoji_drop(&mut self, paths: Vec<std::path::PathBuf>) {
		if self.accepts_server_emoji_drops() {
			self.server_settings.admin.queue_files(paths);
		}
	}
	pub fn take_server_emoji_request(&mut self) -> Option<(u64, Id, u64, Vec<std::path::PathBuf>)> {
		let (generation, guild) = self.server_settings.scope?;
		let paths = self.server_settings.admin.take_files()?;
		self.server_emoji_sequence = self.server_emoji_sequence.wrapping_add(1);
		self.server_settings.admin.request = self.server_emoji_sequence;
		Some((generation, guild, self.server_emoji_sequence, paths))
	}
	pub fn accept_server_emojis(
		&mut self,
		ctx: &egui::Context,
		scope: (u64, Id, u64),
		result: Result<Vec<(String, String, bool, egui::ColorImage)>, &'static str>,
	) {
		if self.server_settings.scope == Some((scope.0, scope.1))
			&& self.server_settings.admin.request == scope.2
		{
			self.server_settings.admin.accept_files(ctx, result);
		}
	}
	pub fn has_server_settings_changes(&self) -> bool {
		self.server_settings.is_open()
			&& (self.server_settings.dirty()
				|| self.server_settings.submitted
				|| self.server_settings.icon_pending)
			|| self.server_settings.admin.has_changes()
			|| self.server_settings.roles.has_changes()
			|| self.server_settings.invites.busy()
			|| self.server_settings.integrations.has_changes()
	}
	/// Opens the same permission-checked editor used by the server menu.
	pub fn preview_server_settings(&mut self, state: &mut State, guild: Id) -> Option<Command> {
		if !state.can_manage_guild(guild) {
			if state.can_open_role_settings(guild) {
				return self.preview_server_roles(state, guild, None);
			}
			if state.can_open_emoji_settings(guild) {
				return self.preview_server_admin(state, guild, "emoji");
			}
			if state.can_open_integration_settings(guild) {
				return self.preview_server_admin(state, guild, "integrations");
			}
			if state.can_open_audit_log_settings(guild) {
				return self.preview_server_admin(state, guild, "audit-log");
			}
			return None;
		}
		self.settings.open = false;
		self.server_settings = Editor {
			scope: Some((state.generation, guild)),
			..Editor::default()
		};
		if state.server_settings.guild == Some(guild) && state.server_settings.snapshot.is_some() {
			None
		} else {
			state.load_server_settings(guild)
		}
	}

	pub fn take_server_icon_request(&mut self) -> Option<(u64, Id, u64)> {
		let editor = &mut self.server_settings;
		if !std::mem::take(&mut editor.icon_requested) {
			return None;
		}
		self.server_icon_sequence = self.server_icon_sequence.wrapping_add(1);
		editor.icon_request = self.server_icon_sequence;
		editor
			.scope
			.map(|(generation, guild)| (generation, guild, editor.icon_request))
	}

	pub fn accept_server_icon(
		&mut self,
		ctx: &egui::Context,
		scope: (u64, Id, u64),
		result: Result<Option<(String, egui::ColorImage)>, &'static str>,
	) {
		let editor = &mut self.server_settings;
		if editor.scope != Some((scope.0, scope.1))
			|| editor.icon_request != scope.2
			|| !editor.icon_pending
		{
			return;
		}
		editor.icon_pending = false;
		match result {
			Ok(Some((data, image)))
				if data.len() <= model::server_settings::MAX_ICON_DATA_URI
					&& image.size[0] <= 512
					&& image.size[1] <= 512 =>
			{
				editor.icon = Patch::Value(data);
				editor.icon_preview = Some(ctx.load_texture(
					"server-icon-draft",
					image,
					egui::TextureOptions::LINEAR,
				));
				editor.icon_error = None;
			}
			Ok(Some(_)) => editor.icon_error = Some("The prepared icon is too large."),
			Ok(None) => {}
			Err(error) => editor.icon_error = Some(error),
		}
	}
}

impl Editor {
	pub fn guild(&self) -> Option<Id> {
		self.scope.map(|(_, guild)| guild)
	}
	pub fn navigate_away(&mut self, state: &mut State) -> bool {
		if !self.is_open() {
			return true;
		}
		if self.integrations.has_changes()
			|| self.roles.has_changes()
			|| self.invites.busy()
			|| self.dirty()
			|| self.admin.has_changes()
			|| state.server_admin.saving
			|| state.server_settings.saving
		{
			self.admin.navigation_error();
			return false;
		}
		*self = Self::default();
		state.close_server_settings();
		state.close_server_admin();
		true
	}
	pub fn is_open(&self) -> bool {
		self.scope.is_some()
	}
	fn dirty(&self) -> bool {
		self.draft != self.baseline || !matches!(self.icon, Patch::Absent)
	}
	fn reset(&mut self) {
		self.draft.clone_from(&self.baseline);
		self.icon = Patch::Absent;
		self.icon_preview = None;
		self.icon_error = None;
		self.form_error = None;
		self.icon_requested = false;
		self.icon_pending = false;
	}
	pub fn show(
		&mut self,
		ctx: &egui::Context,
		state: &mut State,
		avatars: &mut Avatars,
		profile: &mut crate::profiles::ProfileSession,
		commands: &mut Vec<Command>,
	) {
		let Some((generation, guild)) = self.scope else {
			return;
		};
		if generation != state.generation
			|| !(state.can_manage_guild(guild)
				|| state.can_open_emoji_settings(guild)
				|| state.can_open_member_settings(guild)
				|| state.can_open_role_settings(guild)
				|| state.can_open_integration_settings(guild)
				|| state.can_open_audit_log_settings(guild))
			|| !state.guilds.iter().any(|known| known.id == guild)
		{
			*self = Self::default();
			state.close_server_settings();
			state.close_server_admin();
			return;
		}
		if !self.page.allowed(state, guild) {
			if !state.can_manage_guild(guild) {
				self.draft = None;
				self.baseline = None;
				self.icon = Patch::Absent;
				self.icon_preview = None;
				self.icon_pending = false;
				self.icon_requested = false;
			}
			self.page = if state.can_manage_guild(guild) {
				Page::Profile
			} else if state.can_open_role_settings(guild) {
				Page::Roles
			} else if state.can_open_emoji_settings(guild) {
				Page::Emoji
			} else if state.can_open_member_settings(guild) {
				Page::Members
			} else if state.can_open_integration_settings(guild) {
				Page::Integrations
			} else {
				Page::AuditLog
			};
			self.admin = crate::server_admin::Admin::default();
			self.roles = crate::server_roles::RolesUi::default();
			self.invites = crate::server_invites::InvitesUi::default();
			self.integrations = crate::server_integrations::IntegrationsUi::default();
			self.audit_log = crate::server_audit_log::AuditLogUi::default();
			state.close_server_admin();
		}
		self.invites.sync(state, guild);
		if self.page == Page::AuditLog
			&& let Some(command) = self.audit_log.load(state, guild)
		{
			commands.push(command);
		}
		self.integrations.sync(state, guild);
		if self.page == Page::Integrations
			&& let Some(command) = self.integrations.load(state, guild)
		{
			commands.push(command);
		}
		if self.page == Page::Invites
			&& let Some(command) = self.invites.load(state, guild)
		{
			commands.push(command);
		}
		if self.page == Page::Roles
			&& let Some(command) = self.roles.load(state, guild)
		{
			commands.push(command);
		}
		if matches!(self.page, Page::Emoji | Page::Members)
			&& let Some(command) = self.admin.load(state, guild, self.page == Page::Members)
		{
			commands.push(command);
		}
		if state.server_settings.guild == Some(guild) {
			if self.submitted && !state.server_settings.pending {
				self.submitted = false;
				if state.server_settings.error.is_none() {
					self.baseline.clone_from(&state.server_settings.snapshot);
					self.reset();
				}
			}
			if (self.draft.is_none() || self.revision != state.server_settings.revision)
				&& let Some(snapshot) = &state.server_settings.snapshot
			{
				let changes = self
					.baseline
					.as_ref()
					.zip(self.draft.as_ref())
					.map(|(before, draft)| Edit::between(before, draft));
				let mut draft = snapshot.clone();
				if let Some(changes) = changes {
					changes.apply(&mut draft);
				}
				self.draft = Some(draft);
				self.baseline = Some(snapshot.clone());
				self.revision = state.server_settings.revision;
			}
		}
		let colors = design::palette_for(ctx);
		let size = ctx.content_rect().size();
		let width = (size.x - 32.0).clamp(260.0, 1160.0);
		let height = (size.y - 32.0).max(220.0);
		let wide = width >= 720.0;
		let mut close = false;
		let invite_overlay = self.invites.overlay_open() || self.integrations.overlay_open();
		let modal = egui::Modal::new(egui::Id::unique("server-settings"))
			.backdrop_color(dialog::backdrop(ctx))
			.frame(
				egui::Frame::new()
					.fill(colors.chat.to_opaque())
					.stroke(egui::Stroke::new(1.0, colors.border))
					.corner_radius(dialog::RADIUS)
					.shadow(ctx.style_of(ctx.theme()).visuals.window_shadow)
					.inner_margin(0),
			)
			.show(ctx, |ui| {
				ui.set_width(width);
				ui.set_height(height);
				ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
				ui.spacing_mut().item_spacing = egui::vec2(8.0, 8.0);
				if wide {
					egui::Panel::left("server-settings-navigation")
						.exact_size(220.0)
						.resizable(false)
						.show_separator_line(false)
						.frame(
							egui::Frame::new()
								.fill(colors.sidebar.to_opaque())
								.corner_radius(egui::CornerRadius {
									nw: dialog::RADIUS,
									sw: dialog::RADIUS,
									ne: 0,
									se: 0,
								})
								.inner_margin(egui::Margin::symmetric(12, 28)),
						)
						.show(ui, |ui| {
							if self.page == Page::Roles && self.roles.editing() {
								self.roles.navigation(ui, state, guild, commands);
								return;
							}
							let name = state
								.guild(guild)
								.map_or("Server", |known| known.name.as_str());
							ui.label(design::eyebrow(ui, name, colors.muted));
							ui.add_space(12.0);
							for page in [
								Page::Profile,
								Page::Engagement,
								Page::Emoji,
								Page::Members,
								Page::Roles,
								Page::Invites,
								Page::Integrations,
								Page::AuditLog,
							] {
								if !page.allowed(state, guild) {
									continue;
								}
								if matches!(
									page,
									Page::Emoji
										| Page::Members | Page::Integrations
										| Page::AuditLog
								) || (page == Page::Roles
									&& !Page::Members.allowed(state, guild))
								{
									ui.add_space(16.0);
									ui.separator();
									ui.add_space(12.0);
									ui.label(design::eyebrow(
										ui,
										if page == Page::AuditLog {
											"MODERATION"
										} else if page == Page::Integrations {
											"APPS"
										} else if page == Page::Emoji {
											"EXPRESSION"
										} else {
											"PEOPLE"
										},
										colors.muted,
									));
								}
								if crate::settings::nav_item(ui, page.label(), self.page == page)
									.clicked()
								{
									self.page = page;
								}
							}
						});
				}
				egui::CentralPanel::default()
					.frame(egui::Frame::new().inner_margin(egui::Margin {
						left: if wide { 32 } else { 16 },
						right: if wide { 64 } else { 16 },
						top: 40,
						bottom: 24,
					}))
					.show(ui, |ui| {
						if wide {
							let rect = egui::Rect::from_min_size(
								ui.max_rect().right_top() + egui::vec2(16.0, 0.0),
								egui::vec2(40.0, 64.0),
							);
							let mut close_ui = ui.new_child(
								egui::UiBuilder::new()
									.id_salt("server-close")
									.max_rect(rect),
							);
							close = close_control(&mut close_ui).clicked();
						}
						if !wide {
							ui.horizontal_wrapped(|ui| {
								for page in [
									Page::Profile,
									Page::Engagement,
									Page::Emoji,
									Page::Members,
									Page::Roles,
									Page::Invites,
									Page::Integrations,
									Page::AuditLog,
								] {
									if !page.allowed(state, guild) {
										continue;
									}
									ui.selectable_value(&mut self.page, page, page.label());
								}
								close = close_control(ui).clicked();
							});
						}
						if self.dirty() || state.server_settings.saving {
							egui::Panel::bottom("server-settings-save")
								.frame(save_bar_frame(ctx, colors))
								.show(ui, |ui| self.save_bar(ui, state, commands));
						}
						if self.page == Page::Roles && self.roles.has_changes() {
							egui::Panel::bottom("role-settings-save")
								.frame(save_bar_frame(ctx, colors))
								.show(ui, |ui| self.roles.save_bar(ui, state, guild, commands));
						}
						// Pages that virtualize their own list own the only vertical scrollbar;
						// wrapping them again would nest two scroll areas over one list.
						if self.scrolling_page() {
							ui.set_width(ui.available_width());
							self.page_body(ui, state, guild, avatars, profile, commands);
						} else {
							egui::ScrollArea::vertical()
								.id_salt(("server-settings-content", self.page as u8))
								.auto_shrink([false, false])
								.show(ui, |ui| {
									ui.set_width(ui.available_width());
									self.page_body(ui, state, guild, avatars, profile, commands);
									ui.add_space(24.0);
								});
						}
					});
			});
		if self.page == Page::Invites {
			self.invites.overlays(ctx, state, guild, avatars, commands);
		}
		if self.page == Page::Integrations {
			self.integrations.overlays(ctx, state, guild, commands);
		}
		if !invite_overlay && (close || modal.should_close()) {
			if self.dirty()
				|| state.server_settings.saving
				|| self.admin.has_changes()
				|| self.roles.has_changes()
				|| self.integrations.has_changes()
				|| self.invites.busy()
				|| state.server_admin.saving
			{
				self.discard = true;
			} else {
				self.scope = None;
				self.roles = crate::server_roles::RolesUi::default();
				self.invites = crate::server_invites::InvitesUi::default();
				self.integrations = crate::server_integrations::IntegrationsUi::default();
				self.audit_log = crate::server_audit_log::AuditLogUi::default();
				state.close_server_settings();
				state.close_server_admin();
			}
		}
		if self.discard {
			let busy =
				state.server_settings.saving || state.server_admin.saving || self.invites.busy();
			let mut confirmation = dialog::Confirm::new(
				"discard-server-settings",
				"Discard unsaved changes?",
				"Your changes to this server will be lost.",
			)
			.danger()
			.confirm_label("Discard Changes")
			.cancel_label("Keep Editing")
			.enabled(!busy);
			if busy {
				confirmation = confirmation.note(
					dialog::Level::Info,
					"Wait for the current save to finish before closing.",
				);
			}
			match confirmation.show(ctx) {
				Some(dialog::Choice::Confirmed) => {
					*self = Self::default();
					state.close_server_settings();
					state.close_server_admin();
				}
				Some(dialog::Choice::Cancelled) => self.discard = false,
				None => {}
			}
		}
	}

	/// Pages whose own virtualized list scrolls; they must not sit inside a second scroll area.
	fn scrolling_page(&self) -> bool {
		match self.page {
			Page::AuditLog | Page::Invites => true,
			Page::Integrations => self.integrations.scrolls_itself(),
			_ => false,
		}
	}

	/// One page of settings content, with no scroll chrome of its own.
	fn page_body(
		&mut self,
		ui: &mut egui::Ui,
		state: &mut State,
		guild: Id,
		avatars: &mut Avatars,
		profile: &mut crate::profiles::ProfileSession,
		commands: &mut Vec<Command>,
	) {
		match self.page {
			Page::AuditLog => {
				self.audit_log.show(ui, state, guild, avatars, commands);
				return;
			}
			Page::Integrations => {
				self.integrations.show(ui, state, guild, avatars, commands);
				return;
			}
			Page::Invites => {
				self.invites.show(ui, state, guild, avatars, commands);
				return;
			}
			Page::Roles => {
				self.roles.show(ui, state, guild, avatars, commands);
				return;
			}
			Page::Emoji | Page::Members => {
				self.admin.show(
					ui,
					state,
					guild,
					self.page == Page::Members,
					avatars,
					profile,
					commands,
				);
				return;
			}
			Page::Profile | Page::Engagement => {}
		}
		if let Some(error) = state.server_settings.error {
			dialog::notice(ui, dialog::Level::Error, error);
			if !state.server_settings.pending
				&& ui.button("Reload server settings").clicked()
				&& let Some(command) = state.load_server_settings(guild)
			{
				commands.push(command);
			}
		}
		if self.draft.is_none() {
			if state.server_settings.pending {
				ui.horizontal(|ui| {
					ui.spinner();
					ui.label("Loading server settings…");
				});
			} else if !state.gateway_connected && !state.demo {
				ui.weak("Reconnect to load server settings.");
			} else if ui.button("Load server settings").clicked()
				&& let Some(command) = state.load_server_settings(guild)
			{
				commands.push(command);
			}
			return;
		}
		ui.add_enabled_ui(!state.server_settings.pending, |ui| {
			if self.page == Page::Profile {
				self.profile(ui, state, avatars);
			} else if let Some(draft) = &mut self.draft {
				engagement(ui, state, draft);
			}
		});
	}

	fn save_bar(&mut self, ui: &mut egui::Ui, state: &mut State, commands: &mut Vec<Command>) {
		let available = !state.server_settings.pending && !self.icon_pending;
		let (save, reset) = design::save_bar(
			ui,
			state.server_settings.saving.then_some("Saving changes…"),
			available
				&& !state.server_settings.needs_refresh
				&& (state.demo || state.gateway_connected),
			available,
		);
		if reset {
			self.reset();
		}
		if save && let (Some(baseline), Some(draft)) = (&self.baseline, &self.draft) {
			let mut edit = Edit::between(baseline, draft);
			if let Some(traits) = &mut edit.traits {
				traits.retain(|entry| !entry.label.is_empty());
			}
			edit.icon = self.icon.clone();
			if !edit.valid() {
				self.form_error = Some(
					"Use a server name of 2–100 characters, a description of up to 300 characters, and valid traits without control characters.",
				);
				return;
			}
			if let Some(command) = state.save_server_settings(edit) {
				commands.push(command);
				self.submitted = true;
				self.form_error = None;
			} else {
				self.form_error = Some(
					"Could not save these changes. Check the selected channels, reconnect, or reload the server settings and try again.",
				);
			}
		}
		if state.server_settings.needs_refresh {
			ui.add_space(8.0);
			dialog::notice(
				ui,
				dialog::Level::Warning,
				"Reload the server settings before saving again. Your edits will be kept.",
			);
		}
		if !state.demo && !state.gateway_connected {
			ui.add_space(8.0);
			dialog::notice(ui, dialog::Level::Warning, "Reconnect to save changes.");
		}
		if let Some(error) = self.form_error {
			ui.add_space(8.0);
			dialog::notice(ui, dialog::Level::Error, error);
		}
	}

	fn profile(&mut self, ui: &mut egui::Ui, state: &State, avatars: &mut Avatars) {
		let width = ui.available_width();
		if width >= 700.0 {
			let preview_width = if width >= 820.0 { 300.0 } else { 260.0 };
			let form_width = width - preview_width - 32.0;
			ui.horizontal_top(|ui| {
				ui.spacing_mut().item_spacing.x = 32.0;
				ui.allocate_ui_with_layout(
					egui::vec2(form_width, 0.0),
					egui::Layout::top_down(egui::Align::Min),
					|ui| {
						ui.set_width(form_width);
						self.profile_form(ui);
					},
				);
				ui.vertical(|ui| {
					ui.set_width(preview_width);
					self.preview(ui, state, avatars);
				});
			});
		} else {
			self.profile_form(ui);
			ui.add_space(28.0);
			self.preview(ui, state, avatars);
		}
	}
	fn profile_form(&mut self, ui: &mut egui::Ui) {
		let Some(draft) = &mut self.draft else {
			return;
		};
		let colors = design::palette(ui);
		// Column spacing must not leak into the swatch, trait and button rows.
		ui.spacing_mut().item_spacing = egui::vec2(8.0, 8.0);
		ui.label(design::semibold(ui, "Server Profile", 20.0).color(colors.text_strong));
		ui.label("Customize how your server appears in invite links and, if enabled, in Server Discovery and Announcement Channel messages.");
		ui.add_space(24.0);
		let name_label = design::label(ui, "Name");
		design::input(
			ui,
			egui::TextEdit::singleline(&mut draft.name).char_limit(100),
		)
		.labelled_by(name_label.id);
		design::divider(ui);
		design::label(ui, "Icon");
		ui.weak("We recommend an image of at least 512×512.");
		ui.horizontal_wrapped(|ui| {
			if ui
				.add_enabled_ui(!self.icon_pending, |ui| {
					design::button(
						ui,
						if self.icon_pending {
							"Preparing icon…"
						} else {
							"Change Server Icon"
						},
						design::ButtonKind::Primary,
					)
				})
				.inner
				.clicked()
			{
				self.icon_requested = true;
				self.icon_pending = true;
				self.icon_error = None;
			}
			if ui
				.add_enabled_ui(
					draft.icon.is_some() || matches!(self.icon, Patch::Value(_)),
					|ui| design::button(ui, "Remove Icon", design::ButtonKind::Outline),
				)
				.inner
				.clicked()
			{
				self.icon = Patch::Null;
				self.icon_preview = None;
				self.icon_pending = false;
				self.icon_requested = false;
			}
		});
		if let Some(error) = self.icon_error {
			design::notice(ui, design::Level::Error, error);
		}
		design::divider(ui);
		design::label(ui, "Banner");
		let swatches = [
			0x2153dc, 0xf916a0, 0xed171a, 0xef7912, 0xf1cd29, 0x763a94, 0x04adf1, 0x46dcca,
			0x496b00, 0x282828,
		];
		let swatch_width = ((ui.available_width() - 32.0) / 5.0).max(20.0);
		for row in swatches.chunks(5) {
			ui.horizontal(|ui| {
				for &color in row {
					let (rect, response) = ui
						.allocate_exact_size(egui::vec2(swatch_width, 64.0), egui::Sense::click());
					gradient(ui, rect, color, 8);
					if draft.banner_color == Some(color) {
						ui.painter().rect_stroke(
							rect.expand(3.0),
							10,
							egui::Stroke::new(1.5, colors.text),
							egui::StrokeKind::Outside,
						);
					}
					let name = format!("Banner color #{color:06X}");
					response.widget_info(|| {
						egui::WidgetInfo::selected(
							egui::Role::RadioButton,
							ui.is_enabled(),
							draft.banner_color == Some(color),
							&name,
						)
					});
					if response.clicked() {
						draft.banner_color = Some(color);
					}
					response.on_hover_text(name);
				}
			});
		}
		design::divider(ui);
		design::label(ui, "Traits");
		ui.weak("Add up to 5 traits to show off your server's interests and personality.");
		let columns = if ui.available_width() >= 480.0 {
			3
		} else if ui.available_width() >= 330.0 {
			2
		} else {
			1
		};
		let cell_width = (ui.available_width() - 8.0 * (columns - 1) as f32) / columns as f32;
		let mut traits = draft.traits.clone();
		traits.resize_with(5, || Trait {
			label: String::new(),
			emoji: None,
		});
		for (row, cells) in traits.chunks_mut(columns).enumerate() {
			ui.horizontal(|ui| {
				for (column, entry) in cells.iter_mut().enumerate() {
					ui.push_id(("server-trait", row, column), |ui| {
						egui::Frame::new()
							.stroke(egui::Stroke::new(1.0, colors.border))
							.corner_radius(8)
							.inner_margin(8)
							.show(ui, |ui| {
								ui.set_width((cell_width - 18.0).max(60.0));
								ui.horizontal(|ui| {
									self.emoji_picker.unicode_button(ui, &mut entry.emoji);
									ui.add(
										egui::TextEdit::singleline(&mut entry.label)
											.char_limit(100)
											.desired_width((cell_width - 90.0).max(24.0))
											.frame(egui::Frame::NONE),
									)
									.on_hover_text("Trait name");
									if !entry.label.is_empty()
										&& crate::icons::button(
											ui,
											crate::icons::Icon::Close,
											18.0,
											"Remove trait",
										)
										.clicked()
									{
										entry.label.clear();
										entry.emoji = None;
									}
								});
							});
					});
				}
			});
		}
		// Keep empty slots in the editor so an emoji can be chosen before typing its label.
		while traits.last().is_some_and(|entry| {
			entry.label.is_empty() && entry.emoji.as_deref().is_none_or(str::is_empty)
		}) {
			traits.pop();
		}
		for entry in &mut traits {
			if entry.emoji.as_deref() == Some("") {
				entry.emoji = None;
			}
		}
		draft.traits = traits;
		design::divider(ui);
		let description_label = design::label(ui, "Description");
		ui.weak("How did your server get started? Why should people join?");
		design::input(
			ui,
			egui::TextEdit::multiline(&mut draft.description)
				.hint_text("Tell the world a bit about this server.")
				.char_limit(300)
				.desired_width(f32::INFINITY)
				.desired_rows(4),
		)
		.labelled_by(description_label.id);
	}
	fn preview(&self, ui: &mut egui::Ui, state: &State, avatars: &mut Avatars) {
		let Some(draft) = &self.draft else {
			return;
		};
		let colors = design::palette(ui);
		ui.spacing_mut().item_spacing = egui::vec2(6.0, 6.0);
		ui.style_mut().override_font_id = Some(egui::FontId::proportional(13.0));
		let width = ui.available_width().min(300.0) - 2.0;
		egui::Frame::new()
			.fill(colors.raised)
			.stroke(egui::Stroke::new(1.0, colors.border))
			.corner_radius(16)
			.show(ui, |ui| {
				ui.set_width(width);
				let (banner, _) =
					ui.allocate_exact_size(egui::vec2(width, 125.0), egui::Sense::hover());
				gradient(ui, banner, draft.banner_color.unwrap_or(0x2153dc), 16);
				egui::Frame::new().inner_margin(16).show(ui, |ui| {
					ui.set_width(width - 32.0);
					let icon_rect = egui::Rect::from_min_size(
						egui::pos2(ui.cursor().left(), banner.bottom() - 36.0),
						egui::Vec2::splat(72.0),
					);
					ui.painter()
						.rect_filled(icon_rect.expand(4.0), 20, colors.raised);
					if let Some(texture) = &self.icon_preview {
						egui::Image::from_texture(texture)
							.corner_radius(16)
							.paint_at(ui, icon_rect);
					} else if let Some(guild) = state.guild(draft.guild) {
						let mut guild = guild.clone();
						guild.name.clone_from(&draft.name);
						if matches!(self.icon, Patch::Null) {
							guild.icon = None;
						}
						ui.scope_builder(egui::UiBuilder::new().max_rect(icon_rect), |ui| {
							avatars.show_guild_sized(ui, &guild, true, state.demo, 72.0);
						});
					}
					ui.advance_cursor_after_rect(icon_rect);
					ui.label(design::semibold(ui, &draft.name, 16.0));
					ui.horizontal_wrapped(|ui| {
						if let Some(count) = draft.online_count {
							ui.colored_label(colors.positive, format!("● {count} Online"));
						}
						if let Some(count) = draft.member_count {
							ui.weak(format!("● {count} Members"));
						}
					});
					let seconds = ((draft.guild.0 >> 22) + 1_420_070_400_000) / 1000;
					if let Ok(date) = time::OffsetDateTime::from_unix_timestamp(seconds as i64) {
						ui.weak(format!("Est. {} {}", date.month(), date.year()));
					}
					ui.horizontal_wrapped(|ui| {
						for entry in &draft.traits {
							if !entry.label.is_empty() {
								ui.add(
									egui::Button::new(format!(
										"{}{}{}",
										entry.emoji.as_deref().unwrap_or(""),
										if entry.emoji.is_some() { " " } else { "" },
										entry.label
									))
									.wrap()
									.sense(egui::Sense::hover())
									.fill(Color32::TRANSPARENT)
									.stroke(egui::Stroke::new(1.0, colors.border))
									.corner_radius(20),
								);
							}
						}
					});
					if !draft.description.is_empty() {
						ui.add_space(8.0);
						ui.label(&draft.description);
					}
				});
			});
	}
}

fn gradient(ui: &mut egui::Ui, rect: egui::Rect, color: u32, radius: u8) {
	let top = Color32::from_rgb((color >> 16) as u8, (color >> 8) as u8, color as u8);
	let bottom = top.lerp_to_gamma(Color32::WHITE, 0.38);
	ui.painter().rect_filled(rect, radius, top);
	// Match the profile-card gradient: rounded solid caps and an interpolated middle band.
	ui.painter().rect_filled(
		egui::Rect::from_min_max(
			egui::pos2(rect.left(), rect.bottom() - f32::from(radius)),
			rect.right_bottom(),
		),
		egui::CornerRadius {
			nw: 0,
			ne: 0,
			sw: radius,
			se: radius,
		},
		bottom,
	);
	let band = egui::Rect::from_min_max(
		egui::pos2(rect.left(), rect.top() + f32::from(radius) - 1.0),
		egui::pos2(rect.right(), rect.bottom() - f32::from(radius) + 1.0),
	);
	let mut mesh = egui::Mesh::default();
	mesh.colored_vertex(band.left_top(), top);
	mesh.colored_vertex(band.right_top(), top);
	mesh.colored_vertex(band.left_bottom(), bottom);
	mesh.colored_vertex(band.right_bottom(), bottom);
	mesh.add_triangle(0, 1, 2);
	mesh.add_triangle(1, 3, 2);
	ui.painter().add(egui::Shape::mesh(mesh));
}
fn engagement(ui: &mut egui::Ui, state: &State, draft: &mut Settings) {
	ui.set_max_width(850.0);
	ui.spacing_mut().item_spacing.y = 8.0;
	ui.label(design::semibold(ui, "Engagement", 20.0));
	ui.label("Manage settings that help keep your server active.");
	ui.add_space(32.0);
	ui.label(design::semibold(ui, "System Messages", 21.0));
	ui.label("Configure system event messages sent to your server.");
	for (bit, text) in [
		(
			0,
			"Send a random welcome message when someone joins this server.",
		),
		(
			3,
			"Prompt members to reply to welcome messages with a sticker.",
		),
		(1, "Send a message when someone boosts this server."),
		(2, "Send helpful tips for server setup."),
	] {
		let mask = 1 << bit;
		let mut enabled = draft.system_channel_flags & mask == 0;
		if design::switch(ui, text, None, &mut enabled).changed() {
			if enabled {
				draft.system_channel_flags &= !mask;
			} else {
				draft.system_channel_flags |= mask;
			}
		}
	}
	ui.add_space(12.0);
	design::label(ui, "System Messages Channel");
	ui.weak("This is the channel we send system event messages to.");
	channel_picker(ui, state, draft.guild, &mut draft.system_channel_id, false);
	design::divider(ui);
	ui.label(design::semibold(ui, "Activity Feed Settings", 21.0));
	ui.label("Shows a feed of activity from games and connected apps in this server.");
	let mut enabled = draft.activity_feed.unwrap_or(false);
	if design::switch(
		ui,
		"Display Activity Feed in this server",
		None,
		&mut enabled,
	)
	.changed()
	{
		draft.activity_feed = Some(enabled);
	}
	if draft.activity_feed.is_none() {
		ui.weak("Server default");
	}
	design::divider(ui);
	design::label(ui, "Default Notification Settings");
	ui.weak("This will determine whether members who have not explicitly set their notification settings receive a notification for every message sent in this server or not.");
	ui.radio_value(&mut draft.default_message_notifications, 0, "All Messages");
	ui.radio_value(
		&mut draft.default_message_notifications,
		1,
		"Only @mentions",
	);
	ui.weak("We highly recommend setting this to only @mentions for a Community Server.");
	design::divider(ui);
	if ui.available_width() >= 500.0 {
		ui.columns(2, |columns| {
			design::label(&mut columns[0], "Inactive Channel");
			channel_picker(
				&mut columns[0],
				state,
				draft.guild,
				&mut draft.afk_channel_id,
				true,
			);
			design::label(&mut columns[1], "Inactive Timeout");
			columns[1].add_enabled_ui(draft.afk_channel_id.is_some(), |ui| {
				timeout_picker(ui, &mut draft.afk_timeout)
			});
		});
	} else {
		design::label(ui, "Inactive Channel");
		channel_picker(ui, state, draft.guild, &mut draft.afk_channel_id, true);
		design::label(ui, "Inactive Timeout");
		ui.add_enabled_ui(draft.afk_channel_id.is_some(), |ui| {
			timeout_picker(ui, &mut draft.afk_timeout)
		});
	}
	ui.weak("Automatically move members to this channel and mute them when they have been idle for longer than the inactive timeout. This does not affect browsers.");
}
fn channel_picker(
	ui: &mut egui::Ui,
	state: &State,
	guild: Id,
	selected: &mut Option<Id>,
	voice: bool,
) {
	let choices: Vec<_> = state
		.channels
		.iter()
		.filter(|channel| {
			channel.guild == Some(guild)
				&& state.can_view(channel.id)
				&& if voice {
					channel.kind == 2
				} else {
					matches!(channel.kind, 0 | 5)
				}
		})
		.collect();
	let name = selected
		.and_then(|id| choices.iter().find(|channel| channel.id == id))
		.map_or(
			if selected.is_some() {
				"Unavailable channel"
			} else if voice {
				"No Inactive Channel"
			} else {
				"No System Messages Channel"
			},
			|channel| channel.name.as_str(),
		);
	let empty = choices.is_empty();
	egui::ComboBox::from_id_salt(("server-channel", voice))
		.selected_text(name)
		.width(ui.available_width())
		.show_ui(ui, |ui| {
			ui.selectable_value(selected, None, "None");
			for channel in choices {
				ui.selectable_value(
					selected,
					Some(channel.id),
					format!("{} {}", if voice { "♪" } else { "#" }, channel.name),
				);
			}
		});
	if empty {
		ui.weak("No accessible channels available.");
	}
}
fn timeout_picker(ui: &mut egui::Ui, timeout: &mut u32) {
	egui::ComboBox::from_id_salt("server-afk-timeout")
		.selected_text(format!("{} minutes", *timeout / 60))
		.width(ui.available_width())
		.show_ui(ui, |ui| {
			for seconds in [60, 300, 900, 1800, 3600] {
				ui.selectable_value(timeout, seconds, format!("{} minutes", seconds / 60));
			}
		});
}

/// Floating "unsaved changes" strip Discord pins over the settings content.
fn save_bar_frame(ctx: &egui::Context, colors: design::Palette) -> egui::Frame {
	egui::Frame::new()
		.fill(colors.base.to_opaque())
		.stroke(egui::Stroke::new(1.0, colors.border))
		.corner_radius(10)
		.shadow(ctx.style_of(ctx.theme()).visuals.window_shadow)
		.inner_margin(egui::Margin::symmetric(14, 12))
		.outer_margin(egui::Margin {
			left: 0,
			right: 0,
			top: 8,
			bottom: 8,
		})
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn profile_columns_stay_inside_available_width() {
		for width in [360.0, 700.0, 844.0] {
			let ctx = egui::Context::default();
			design::apply(&ctx);
			let state = test_support::demo_state();
			let settings = Settings {
				guild: state.guilds[0].id,
				name: "A long synthetic server name ".repeat(3),
				description: "A long description that wraps within the form and preview. "
					.repeat(4),
				traits: (0..5)
					.map(|_| Trait {
						label: "A long trait".repeat(7),
						emoji: None,
					})
					.collect(),
				..Default::default()
			};
			let mut editor = Editor {
				draft: Some(settings),
				..Default::default()
			};
			let mut avatars = Avatars::default();
			let output = ctx.run_ui(
				egui::RawInput {
					screen_rect: Some(egui::Rect::from_min_size(
						egui::Pos2::ZERO,
						egui::vec2(width, 1800.0),
					)),
					..Default::default()
				},
				|ui| {
					ui.set_width(width);
					ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
					let right = ui.max_rect().right();
					editor.profile(ui, &state, &mut avatars);
					assert!(
						ui.min_rect().right() <= right + 1.0,
						"profile overflow at {width}: {:?}",
						ui.min_rect()
					);
				},
			);
			output.drop_without_applying_deltas();
		}
	}
}
