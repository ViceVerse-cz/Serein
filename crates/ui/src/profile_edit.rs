//! One bounded, session-only draft for the account's global profile.
use crate::{avatars::Avatars, design, icons};
use client_core::{Command, State};
use model::{Id, ProfileEdit, UserProfile};

#[derive(Clone, PartialEq, Eq)]
struct Draft {
	name: String,
	bio: String,
	pronouns: String,
	color: Option<u32>,
	/// `Some(Some(uri))` uploads a new picture, `Some(None)` removes the current one.
	avatar: Option<Option<String>>,
}
impl Draft {
	fn from_profile(profile: &UserProfile) -> Self {
		Self {
			name: profile.global_name.clone().unwrap_or_default(),
			bio: profile.bio.clone(),
			pronouns: profile.pronouns.clone(),
			color: profile.accent_color,
			avatar: None,
		}
	}
	fn changes(&self, profile: &UserProfile) -> ProfileEdit {
		let name = (!self.name.is_empty()).then(|| self.name.clone());
		ProfileEdit {
			global_name: (name != profile.global_name).then_some(name),
			bio: (self.bio != profile.bio).then(|| self.bio.clone()),
			pronouns: (self.pronouns != profile.pronouns).then(|| self.pronouns.clone()),
			accent_color: (self.color != profile.accent_color).then_some(self.color),
			avatar: match &self.avatar {
				// Removing a picture the account does not have is not a change.
				Some(None) if profile.user.avatar.is_none() => None,
				other => other.clone(),
			},
		}
	}
	fn rebase(&mut self, before: &Self, fresh: &Self) {
		if self.name == before.name {
			self.name = fresh.name.clone();
		}
		if self.bio == before.bio {
			self.bio = fresh.bio.clone();
		}
		if self.pronouns == before.pronouns {
			self.pronouns = fresh.pronouns.clone();
		}
		if self.color == before.color {
			self.color = fresh.color;
		}
	}
}

/// What the preview card paints inside the avatar circle.
enum Picture<'a> {
	/// The account's current picture, fetched like any other avatar.
	Remote,
	/// A locally chosen image that has not been saved yet.
	Pending(&'a egui::TextureHandle),
	/// A locally chosen image that was just saved; shown until the CDN copy is fetched.
	Saved(&'a egui::TextureHandle),
	/// Removal is pending, so the initial placeholder is shown.
	Removed,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum AvatarAction {
	Pick,
	Remove,
	Undo,
}

#[derive(Default)]
pub(super) struct Editor {
	generation: Option<u64>,
	draft: Option<Draft>,
	baseline: Option<Draft>,
	submitted: Option<u64>,
	saved: bool,
	preview_link: Option<String>,
	/// Set when the picture button is pressed; the desktop shell opens the native picker.
	pub avatar_request: Option<(u64, Id, u64)>,
	revision: u64,
	choosing: bool,
	pending_picture: Option<egui::TextureHandle>,
	/// Local copy of the last saved picture, keyed by the avatar hash the service returned.
	saved_picture: Option<(Option<String>, egui::TextureHandle)>,
}
impl Editor {
	/// Result of the native picker. Errors are returned so the caller can toast them.
	pub fn accept_avatar(
		&mut self,
		ctx: &egui::Context,
		request: (u64, Id, u64),
		result: Result<Option<(String, egui::ColorImage)>, &'static str>,
	) -> Result<(), &'static str> {
		if !self.choosing
			|| request
				!= (
					self.generation.unwrap_or_default(),
					request.1,
					self.revision,
				) {
			return Ok(());
		}
		self.choosing = false;
		match result {
			Ok(Some((uri, image))) => {
				if !model::valid_avatar_uri(&uri)
					|| image.resolution()[0] > 512
					|| image.resolution()[1] > 512
				{
					return Err("That picture is too large; choose a simpler image");
				}
				let Some(draft) = self.draft.as_mut() else {
					return Ok(());
				};
				draft.avatar = Some(Some(uri));
				self.pending_picture = Some(ctx.load_texture(
					"profile-avatar-preview",
					image,
					egui::TextureOptions::LINEAR,
				));
				self.saved = false;
				Ok(())
			}
			Ok(None) => Ok(()),
			Err(error) => Err(error),
		}
	}

	pub fn show(
		&mut self,
		ui: &mut egui::Ui,
		state: &mut State,
		avatars: &mut Avatars,
		commands: &mut Vec<Command>,
	) {
		if self.generation != Some(state.generation) {
			*self = Self {
				generation: Some(state.generation),
				..Self::default()
			};
		}
		if state.own_profile.data.is_none()
			&& !state.own_profile.loading
			&& state.own_profile.error.is_none()
			&& let Some(command) = state.load_own_profile()
		{
			commands.push(command);
		}
		if self.submitted == Some(state.own_profile.request) && !state.own_profile.saving {
			self.submitted = None;
			if state.own_profile.error.is_none()
				&& let Some(profile) = &state.own_profile.data
			{
				let uploaded = self
					.draft
					.as_ref()
					.is_some_and(|draft| matches!(draft.avatar, Some(Some(_))));
				self.saved_picture = match (uploaded, self.pending_picture.take()) {
					(true, Some(texture)) => Some((profile.user.avatar.clone(), texture)),
					_ => None,
				};
				self.draft = Some(Draft::from_profile(profile));
				self.saved = true;
			}
		}
		let colors = design::palette(ui);
		// A reload keeps this line over the form; the first load sketches the form instead.
		if state.own_profile.loading && state.own_profile.data.is_some() {
			ui.horizontal(|ui| {
				ui.add(egui::Spinner::new().size(14.0));
				ui.label(
					egui::RichText::new(crate::i18n::translate(
						"profile-edit-show-loading-your-profile",
					))
					.size(13.0)
					.color(colors.muted),
				);
			});
		}
		if let Some(error) = state.own_profile.error {
			design::notice(ui, design::Level::Error, error);
			if !state.own_profile.loading
				&& !state.own_profile.saving
				&& design::text_action(
					ui,
					&crate::i18n::translate("profile-edit-show-reload-profile"),
				)
				.clicked() && let Some(command) = state.load_own_profile()
			{
				commands.push(command);
			}
		}
		let Some(profile) = state.own_profile.data.as_ref() else {
			if state.own_profile.loading {
				pending_editor(ui);
			}
			return;
		};
		let fresh = Draft::from_profile(profile);
		if let (Some(draft), Some(before)) = (&mut self.draft, &self.baseline) {
			draft.rebase(before, &fresh);
		}
		self.baseline = Some(fresh);
		let draft = self
			.draft
			.get_or_insert_with(|| Draft::from_profile(profile));
		let before = draft.clone();
		let editable = !state.own_profile.loading && !state.own_profile.saving;
		let picture = match &draft.avatar {
			Some(Some(_)) => self
				.pending_picture
				.as_ref()
				.map_or(Picture::Remote, Picture::Pending),
			Some(None) => Picture::Removed,
			None => match &self.saved_picture {
				Some((hash, texture)) if hash.is_some() && *hash == profile.user.avatar => {
					Picture::Saved(texture)
				}
				_ => Picture::Remote,
			},
		};
		let mut action = None;
		let width = ui.available_width();
		if let Some(form_width) = columns(width) {
			ui.horizontal_top(|ui| {
				ui.spacing_mut().item_spacing.x = COLUMN_GAP;
				ui.allocate_ui_with_layout(
					egui::vec2(form_width, 0.0),
					egui::Layout::top_down(egui::Align::Min),
					|ui| {
						ui.add_enabled_ui(editable, |ui| {
							action = form(ui, draft, profile, self.choosing);
						});
					},
				);
				ui.vertical(|ui| {
					ui.set_width(PREVIEW_WIDTH);
					if let Some(picked) = preview(
						ui,
						draft,
						profile,
						avatars,
						(state.demo, &state.guilds),
						&mut self.preview_link,
						picture,
						editable && !self.choosing,
					) {
						action = Some(picked);
					}
				});
			});
		} else {
			ui.add_enabled_ui(editable, |ui| {
				action = form(ui, draft, profile, self.choosing);
			});
		}
		match action {
			Some(AvatarAction::Pick) if editable && !self.choosing => {
				self.revision = self.revision.wrapping_add(1);
				self.choosing = true;
				self.avatar_request = Some((state.generation, profile.user.id, self.revision));
			}
			Some(AvatarAction::Remove) => {
				draft.avatar = Some(None);
				self.pending_picture = None;
			}
			Some(AvatarAction::Undo) => {
				draft.avatar = None;
				self.pending_picture = None;
			}
			_ => {}
		}
		if *draft != before {
			self.saved = false;
		}
		let changes = draft.changes(profile);
		let changed = changes != ProfileEdit::default();
		if !changes.valid() {
			design::notice(
				ui,
				design::Level::Error,
				&crate::i18n::translate(
					"profile-edit-show-check-character-limits-and-remove-control-characters-a-display-name",
				),
			);
		}
		ui.add_space(16.0);
		egui::Frame::new()
			.fill(colors.base)
			.corner_radius(8)
			.inner_margin(egui::Margin::symmetric(12, 8))
			.show(ui, |ui| {
				ui.set_width(ui.available_width());
				ui.horizontal(|ui| {
					ui.spacing_mut().item_spacing.x = 12.0;
					if state.own_profile.saving {
						ui.add(egui::Spinner::new().size(14.0));
						ui.label(
							egui::RichText::new(crate::i18n::translate(
								"profile-edit-show-saving-profile",
							))
							.size(13.0)
							.color(colors.muted),
						);
					} else if self.saved {
						ui.label(
							egui::RichText::new(crate::i18n::translate_if_key(if state.demo {
								"profile-edit-show-saved-in-preview"
							} else {
								"profile-edit-show-profile-saved"
							}))
							.size(13.0)
							.color(colors.positive),
						);
					} else if changed {
						ui.label(
							egui::RichText::new(crate::i18n::translate(
								"profile-edit-show-you-have-unsaved-changes",
							))
							.size(13.0)
							.color(colors.text),
						);
					}
					ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
						ui.add_enabled_ui(
							changed && changes.valid() && state.can_save_own_profile(),
							|ui| {
								if design::button(
									ui,
									&crate::i18n::translate("profile-edit-show-save-changes"),
									design::ButtonKind::Primary,
								)
								.clicked() && let Some(command) = state.save_own_profile(changes)
								{
									self.submitted = Some(state.own_profile.request);
									self.saved = false;
									commands.push(command);
								}
							},
						);
						ui.add_enabled_ui(changed && editable, |ui| {
							if design::button(
								ui,
								&crate::i18n::translate("profile-edit-show-cancel"),
								design::ButtonKind::Neutral,
							)
							.clicked()
							{
								self.draft =
									state.own_profile.data.as_ref().map(Draft::from_profile);
								self.pending_picture = None;
								self.saved = false;
							}
						});
					});
				});
			});
		if columns(width).is_none()
			&& let (Some(draft), Some(profile)) = (&self.draft, &state.own_profile.data)
		{
			ui.add_space(16.0);
			let picture = match &draft.avatar {
				Some(Some(_)) => self
					.pending_picture
					.as_ref()
					.map_or(Picture::Remote, Picture::Pending),
				Some(None) => Picture::Removed,
				None => Picture::Remote,
			};
			preview(
				ui,
				draft,
				profile,
				avatars,
				(state.demo, &state.guilds),
				&mut self.preview_link,
				picture,
				false,
			);
		}
		crate::markdown::confirm_external_link(ui.ctx(), &mut self.preview_link, true);
		if !state.demo && !state.gateway_connected {
			design::hint(
				ui,
				&crate::i18n::translate("profile-edit-show-reconnect-to-save-your-profile"),
			);
		}
	}
}

fn form(
	ui: &mut egui::Ui,
	draft: &mut Draft,
	profile: &UserProfile,
	choosing: bool,
) -> Option<AvatarAction> {
	ui.spacing_mut().item_spacing.y = 6.0;
	let mut action = None;
	let has_picture = match &draft.avatar {
		Some(value) => value.is_some(),
		None => profile.user.avatar.is_some(),
	};
	design::row(
		ui,
		"profile-edit-form-profile-picture",
		Some(match &draft.avatar {
			Some(Some(_)) => "profile-edit-form-new-picture-chosen-save-to-upload-it",
			Some(None) => "profile-edit-form-your-picture-will-be-removed-when-you-save",
			None => "profile-edit-form-png-jpeg-gif-or-webp-up-to-8-mb-cropped",
		}),
		|ui| {
			ui.add_enabled_ui(!choosing, |ui| {
				if design::button(
					ui,
					&crate::i18n::translate("profile-edit-form-change"),
					design::ButtonKind::Outline,
				)
				.clicked()
				{
					action = Some(AvatarAction::Pick);
				}
			});
			if draft.avatar.is_some() {
				if design::text_action(ui, &crate::i18n::translate("profile-edit-form-undo"))
					.clicked()
				{
					action = Some(AvatarAction::Undo);
				}
			} else if has_picture
				&& design::text_action(ui, &crate::i18n::translate("profile-edit-form-remove"))
					.clicked()
			{
				action = Some(AvatarAction::Remove);
			}
			if choosing {
				ui.add(egui::Spinner::new().size(14.0));
			}
		},
	);
	ui.add_space(10.0);
	field(
		ui,
		"profile-edit-form-display-name",
		"profile-display-name",
		&mut draft.name,
		model::MAX_PROFILE_NAME_CHARS,
		false,
	);
	name_hint(ui);
	ui.add_space(8.0);
	field(
		ui,
		"profile-edit-form-pronouns",
		"profile-pronouns",
		&mut draft.pronouns,
		model::MAX_PROFILE_PRONOUNS_CHARS,
		false,
	);
	ui.add_space(8.0);
	field(
		ui,
		"profile-edit-form-about-me",
		"profile-about-me",
		&mut draft.bio,
		model::MAX_PROFILE_BIO_CHARS,
		true,
	);
	ui.add_space(8.0);
	let mut enabled = draft.color.is_some();
	design::row(
		ui,
		"profile-edit-form-profile-color",
		Some("profile-edit-form-tints-your-banner-when-you-have-not-set-a-banner"),
		|ui| {
			if let Some(color) = &mut draft.color {
				let mut rgb = [(*color >> 16) as u8, (*color >> 8) as u8, *color as u8];
				if design::color_edit(ui, &mut rgb)
					.on_hover_text(crate::i18n::translate(
						"profile-edit-form-choose-profile-color",
					))
					.changed()
				{
					*color =
						(u32::from(rgb[0]) << 16) | (u32::from(rgb[1]) << 8) | u32::from(rgb[2]);
				}
				if design::text_action(ui, &crate::i18n::translate("profile-edit-form-use-default"))
					.clicked()
				{
					enabled = false;
				}
			} else if design::text_action(
				ui,
				&crate::i18n::translate("profile-edit-form-custom-color"),
			)
			.clicked()
			{
				enabled = true;
			}
		},
	);
	if enabled != draft.color.is_some() {
		// Start from the profile's own colors rather than the app accent.
		draft.color = enabled.then(|| {
			profile
				.accent_color
				.or(profile.theme_colors.map(|[top, _]| top))
				.unwrap_or(design::DEFAULT_PRIMARY_RGB)
		});
	}
	action
}

fn field(
	ui: &mut egui::Ui,
	label: &str,
	id: &str,
	value: &mut String,
	limit: usize,
	multiline: bool,
) {
	let colors = design::palette(ui);
	let counter = multiline.then(|| format!("{} / {limit}", value.chars().count()));
	let label_id = field_label(ui, label, counter);
	let edit = if multiline {
		egui::TextEdit::multiline(value).desired_rows(BIO_ROWS)
	} else {
		egui::TextEdit::singleline(value).align(egui::Align2::LEFT_CENTER)
	};
	design::input(
		ui,
		edit.id(egui::Id::unique(id))
			.char_limit(limit)
			.font(field_font())
			.text_color(colors.text),
	)
	.labelled_by(label_id);
}

/// Width of the preview column beside the form, and the gap between them.
const PREVIEW_WIDTH: f32 = 300.0;
const COLUMN_GAP: f32 = 24.0;
/// The preview's banner height, avatar size and the least height of its details panel.
const BANNER: f32 = 90.0;
const AVATAR: f32 = 80.0;
/// The card-coloured ring behind the preview avatar.
const AVATAR_RING: f32 = AVATAR * 0.5 + 6.0;
/// The preview banner is the card's top: only its upper corners are rounded.
const BANNER_CORNERS: egui::CornerRadius = egui::CornerRadius {
	nw: 12,
	ne: 12,
	sw: 0,
	se: 0,
};
const PANEL_MIN_HEIGHT: f32 = 138.0;
/// Text rows the About Me field shows.
const BIO_ROWS: usize = 4;

/// The form column's width when the preview fits beside it, or `None` when it stacks below.
fn columns(width: f32) -> Option<f32> {
	(width >= 620.0).then_some(width - PREVIEW_WIDTH - COLUMN_GAP)
}

fn field_font() -> egui::FontId {
	egui::FontId::proportional(15.0)
}

/// A field's uppercase label row, with its character counter on the right when given.
fn field_label(ui: &mut egui::Ui, label: &str, counter: Option<String>) -> egui::Id {
	let colors = design::palette(ui);
	let label = crate::i18n::translate_if_key(label);
	ui.horizontal(|ui| {
		let label = ui.label(design::eyebrow(ui, &label, colors.muted));
		if let Some(counter) = counter {
			ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
				ui.label(egui::RichText::new(counter).size(11.0).color(colors.muted));
			});
		}
		label.id
	})
	.inner
}

fn name_hint(ui: &mut egui::Ui) {
	ui.label(
		egui::RichText::new(crate::i18n::translate(
			"profile-edit-form-leave-blank-to-use-your-username",
		))
		.size(12.0)
		.color(design::palette(ui).muted),
	);
}

fn preview_heading(ui: &mut egui::Ui) {
	ui.label(design::eyebrow(
		ui,
		crate::i18n::translate("profile-edit-preview-preview"),
		design::palette(ui).muted,
	));
	ui.add_space(4.0);
}

/// The editor while the profile loads: the form's real titles, labels and hints over input
/// placeholders, and beside it on wide pages the preview's banner, avatar and name lines.
fn pending_editor(ui: &mut egui::Ui) {
	let label = crate::i18n::translate("profile-edit-show-loading-your-profile");
	let width = ui.available_width();
	if let Some(form_width) = columns(width) {
		ui.horizontal_top(|ui| {
			ui.spacing_mut().item_spacing.x = COLUMN_GAP;
			ui.allocate_ui_with_layout(
				egui::vec2(form_width, 0.0),
				egui::Layout::top_down(egui::Align::Min),
				|ui| pending_form(ui, &label),
			);
			ui.vertical(|ui| {
				ui.set_width(PREVIEW_WIDTH);
				pending_preview(ui);
			});
		});
	} else {
		pending_form(ui, &label);
	}
}

fn pending_form(ui: &mut egui::Ui, label: &str) {
	use crate::skeleton::form as pending;
	ui.spacing_mut().item_spacing.y = 6.0;
	design::row(
		ui,
		"profile-edit-form-profile-picture",
		Some("profile-edit-form-png-jpeg-gif-or-webp-up-to-8-mb-cropped"),
		|ui| pending::controls(ui, label, &[92.0], design::BUTTON_HEIGHT),
	);
	ui.add_space(10.0);
	for (index, (key, rows)) in [
		("profile-edit-form-display-name", 1),
		("profile-edit-form-pronouns", 1),
		("profile-edit-form-about-me", BIO_ROWS),
	]
	.into_iter()
	.enumerate()
	{
		if index > 0 {
			ui.add_space(8.0);
		}
		field_label(ui, key, None);
		pending::input(ui, "", Some(field_font()), rows);
		if index == 0 {
			name_hint(ui);
		}
	}
	ui.add_space(8.0);
	design::row(
		ui,
		"profile-edit-form-profile-color",
		Some("profile-edit-form-tints-your-banner-when-you-have-not-set-a-banner"),
		|ui| {
			let height = ui.spacing().interact_size.y;
			pending::controls(ui, "", &[96.0], height);
		},
	);
}

/// The preview card's banner, avatar and details panel with the name and username lines.
fn pending_preview(ui: &mut egui::Ui) {
	use crate::skeleton::{bar, circle, frame_tint, outline};
	use egui::{pos2, vec2};
	preview_heading(ui);
	let page = design::palette(ui).chat;
	let (name, username) = ui.fonts_mut(|fonts| {
		(
			fonts.row_height(&egui::FontId::new(20.0, design::semibold_family(ui.ctx()))),
			fonts.row_height(&egui::FontId::proportional(13.0)),
		)
	});
	// The panel starts 12px under the avatar, which hangs half below the banner, and pads
	// its lines by 12px; the card adds 12px under it.
	let panel_top = BANNER + AVATAR * 0.5 + 12.0;
	let panel = PANEL_MIN_HEIGHT + 24.0;
	crate::skeleton::custom(
		ui,
		"",
		vec2(ui.available_width(), panel_top + panel + 12.0),
		|painter, rect, fill| {
			let width = rect.width();
			// One card frames the banner and the inset panel, as the loaded preview does;
			// without it the panel's inset reads as misalignment.
			outline(painter, fill, rect, 12.0);
			// The banner is the card's top, rounded like the loaded card's.
			outline(
				painter,
				fill,
				egui::Rect::from_min_size(rect.min, vec2(width, BANNER)),
				BANNER_CORNERS,
			);
			// A ring in the card's own colour sets the avatar off the banner, as on the
			// loaded card: the page colour under the card's frame tint.
			let avatar = pos2(rect.left() + 16.0 + AVATAR * 0.5, rect.top() + BANNER);
			circle(painter, page, avatar, AVATAR_RING);
			circle(painter, frame_tint(fill), avatar, AVATAR_RING);
			circle(painter, fill, avatar, AVATAR * 0.5);
			let panel = egui::Rect::from_min_size(
				rect.min + vec2(12.0, panel_top),
				vec2(width - 24.0, panel),
			);
			outline(painter, fill, panel, 8.0);
			let text = panel.min + vec2(12.0, 12.0);
			bar(
				painter,
				fill,
				text + vec2(0.0, (name - 12.0) * 0.5),
				vec2(width * 0.5, 12.0),
			);
			bar(
				painter,
				fill,
				text + vec2(0.0, name + 4.0 + (username - 10.0) * 0.5),
				vec2(width * 0.35, 10.0),
			);
		},
	);
}

#[allow(clippy::too_many_arguments)]
fn preview(
	ui: &mut egui::Ui,
	draft: &Draft,
	profile: &UserProfile,
	avatars: &mut Avatars,
	media: (bool, &[model::Guild]),
	opening: &mut Option<String>,
	picture: Picture<'_>,
	clickable: bool,
) -> Option<AvatarAction> {
	let (demo, guilds) = media;
	let colors = design::palette(ui);
	let mut action = None;
	preview_heading(ui);
	// Paint with the profile's own colors, as other people see the card, not the app theme.
	let theme = crate::profiles::Theme::new(&colors, profile.theme_colors);
	let rgb = |c: u32| egui::Color32::from_rgb((c >> 16) as u8, (c >> 8) as u8, c as u8);
	ui.scope(|ui| {
		let background = ui.painter().add(egui::Shape::Noop);
		ui.visuals_mut().override_text_color = Some(theme.text);
		ui.visuals_mut().hyperlink_color = theme.link;
		ui.set_width(ui.available_width());
		ui.spacing_mut().item_spacing.y = 0.0;
		{
			let (banner, _) = ui.allocate_exact_size(
				egui::vec2(ui.available_width(), BANNER),
				egui::Sense::hover(),
			);
			let corner = BANNER_CORNERS;
			if profile.banner.is_some() {
				// The banner image covers its color, so the saved profile paints it as is.
				avatars.paint_banner(ui, profile, banner, corner, demo);
			} else {
				let color = profile
					.theme_colors
					.map(crate::profiles::Theme::banner)
					.or_else(|| draft.color.map(rgb))
					.unwrap_or(colors.raised);
				ui.painter().rect_filled(banner, corner, color);
			}
			let avatar = egui::Rect::from_min_size(
				banner.left_bottom() + egui::vec2(16.0, -AVATAR * 0.5),
				egui::Vec2::splat(AVATAR),
			);
			ui.painter()
				.circle_filled(avatar.center(), AVATAR_RING, theme.card);
			let response =
				ui.scope_builder(
					egui::UiBuilder::new().max_rect(avatar),
					|ui| match picture {
						Picture::Pending(texture) | Picture::Saved(texture) => ui.add(
							egui::Image::new(texture)
								.fit_to_exact_size(egui::Vec2::splat(80.0))
								.corner_radius(40.0),
						),
						Picture::Removed => design::avatar(ui, &profile.user.name, 80.0),
						Picture::Remote => avatars.with_avatar_animation(true, |avatars| {
							avatars.show_plain(ui, &profile.user, 80.0, demo)
						}),
					},
				);
			if clickable {
				let hit = ui.interact(
					avatar,
					ui.scope_id().with("change-avatar"),
					egui::Sense::click(),
				);
				let hit = hit.on_hover_text(crate::i18n::translate(
					"profile-edit-preview-change-profile-picture",
				));
				if hit.hovered() || hit.has_focus() {
					ui.painter().circle_filled(
						avatar.center(),
						40.0,
						egui::Color32::from_black_alpha(120),
					);
					icons::paint(
						ui.painter(),
						icons::Icon::Pencil,
						egui::Rect::from_center_size(avatar.center(), egui::Vec2::splat(22.0)),
						egui::Color32::WHITE,
					);
				}
				if hit.clicked() {
					action = Some(AvatarAction::Pick);
				}
			}
			drop(response);
			ui.add_space((avatar.bottom() + 12.0 - ui.cursor().top()).max(0.0));
			egui::Frame::new()
				.inner_margin(egui::Margin {
					left: 12,
					right: 12,
					top: 0,
					bottom: 12,
				})
				.show(ui, |ui| {
					egui::Frame::new()
						.fill(theme.panel)
						.corner_radius(8)
						.inner_margin(12)
						.show(ui, |ui| {
							ui.set_width(ui.available_width());
							ui.set_min_height(PANEL_MIN_HEIGHT);
							ui.spacing_mut().item_spacing.y = 4.0;
							let name = if draft.name.is_empty() {
								&profile.username
							} else {
								&draft.name
							};
							ui.add(
								egui::Label::new(
									design::semibold(ui, name, 20.0).color(theme.text),
								)
								.wrap(),
							);
							ui.label(
								egui::RichText::new(&profile.username)
									.size(13.0)
									.color(theme.text),
							);
							if !draft.pronouns.is_empty() {
								ui.label(
									egui::RichText::new(&draft.pronouns)
										.size(12.0)
										.color(theme.muted),
								);
							}
							if !draft.bio.is_empty() {
								ui.add_space(8.0);
								ui.separator();
								ui.add_space(8.0);
								ui.label(design::eyebrow(
									ui,
									crate::i18n::translate("profile-edit-preview-about-me"),
									theme.text,
								));
								let mut mentions = crate::profiles::ProfileSession::default();
								crate::markdown::Formatted::parse(&draft.bio).show_with_images(
									ui,
									opening,
									&[],
									None,
									&mut mentions,
									(avatars, demo, guilds),
									crate::design::MessageCardSurface::Opaque,
								);
							}
						});
				});
		}
		let rect = ui.min_rect();
		ui.painter().set(background, theme.background(rect));
	});
	action
}

#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn draft_only_sends_changed_fields_and_can_clear_values() {
		let user = model::User {
			id: model::Id(1),
			name: "Synthetic".into(),
			avatar: None,
			webhook: false,
			kind: Default::default(),
			discriminator: 0,
			primary_guild: None,
			cosmetics: None,
		};
		let mut profile = crate::profiles::synthetic(&user, None);
		profile.global_name = Some("Before".into());
		profile.accent_color = Some(0x123456);
		let mut draft = Draft::from_profile(&profile);
		assert!(draft.changes(&profile) == ProfileEdit::default());
		draft.name.clear();
		draft.bio.clear();
		draft.color = None;
		let changes = draft.changes(&profile);
		assert_eq!(changes.global_name, Some(None));
		assert_eq!(changes.bio, Some(String::new()));
		assert_eq!(changes.accent_color, Some(None));
		assert_eq!(changes.pronouns, None);
		assert!(changes.valid());
		// A reload retains edited values but adopts remote changes to untouched fields.
		let before = Draft::from_profile(&profile);
		profile.pronouns = "she/her".into();
		draft.rebase(&before, &Draft::from_profile(&profile));
		assert_eq!(draft.pronouns, "she/her");
		assert!(draft.name.is_empty() && draft.bio.is_empty());
		assert_eq!(draft.changes(&profile).pronouns, None);
	}
	#[test]
	fn editor_saves_once_preserves_failed_draft_and_cancels_in_both_themes() {
		fn frame(
			ctx: &egui::Context,
			editor: &mut Editor,
			state: &mut State,
			avatars: &mut Avatars,
			width: f32,
			events: Vec<egui::Event>,
		) -> (Vec<Command>, Vec<(String, egui::Rect)>) {
			let mut commands = vec![];
			let mut output = ctx.run_ui(
				egui::RawInput {
					screen_rect: Some(egui::Rect::from_min_size(
						egui::Pos2::ZERO,
						egui::vec2(width, 1100.0),
					)),
					events,
					..Default::default()
				},
				|ui| editor.show(ui, state, avatars, &mut commands),
			);
			output.textures_delta.clear();
			fn text(shape: &egui::Shape, labels: &mut Vec<(String, egui::Rect)>) {
				match shape {
					egui::Shape::Text(text) => labels.push((
						text.galley.job.text.clone(),
						text.galley.rect.translate(text.pos.to_vec2()),
					)),
					egui::Shape::Vec(shapes) => {
						for shape in shapes {
							text(shape, labels);
						}
					}
					_ => {}
				}
			}
			let mut labels = vec![];
			for shape in output.shapes {
				text(&shape.shape, &mut labels);
			}
			(commands, labels)
		}
		for theme in [egui::ThemePreference::Dark, egui::ThemePreference::Light] {
			for width in [320.0, 720.0] {
				let ctx = egui::Context::default();
				ctx.set_theme(theme);
				let mut editor = Editor::default();
				let mut avatars = Avatars::default();
				let mut state = test_support::demo_state();
				let (commands, _) =
					frame(&ctx, &mut editor, &mut state, &mut avatars, width, vec![]);
				assert_eq!(commands.len(), 1);
				let Command::EditProfile {
					user,
					request,
					changes: None,
				} = commands.into_iter().next().unwrap()
				else {
					panic!("load expected")
				};
				state.apply(client_core::Envelope {
					generation: state.generation,
					event: client_core::Event::ProfileEdited {
						user,
						request,
						result: Ok(Box::new(crate::profiles::synthetic(
							state.user.as_ref().unwrap(),
							None,
						))),
					},
				});
				frame(&ctx, &mut editor, &mut state, &mut avatars, width, vec![]);
				ctx.memory_mut(|memory| {
					memory.request_focus(egui::Id::unique("profile-display-name"))
				});
				frame(
					&ctx,
					&mut editor,
					&mut state,
					&mut avatars,
					width,
					vec![egui::Event::Text(" Edited".into())],
				);
				let draft = editor.draft.as_ref().unwrap().clone();
				assert!(draft.name.contains("Edited"));
				let (_, labels) = frame(&ctx, &mut editor, &mut state, &mut avatars, width, vec![]);
				let save = labels
					.iter()
					.find(|(text, _)| text == "Save changes")
					.unwrap()
					.1;
				assert!(save.left() >= 0.0 && save.right() <= width);
				let mut sent = vec![];
				for pressed in [true, false] {
					let pos = save.center();
					let (commands, _) = frame(
						&ctx,
						&mut editor,
						&mut state,
						&mut avatars,
						width,
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
					sent.extend(commands);
				}
				assert_eq!(sent.len(), 1);
				assert!(
					frame(&ctx, &mut editor, &mut state, &mut avatars, width, vec![])
						.0
						.is_empty()
				);
				let Command::EditProfile {
					user,
					request,
					changes: Some(changes),
				} = sent.pop().unwrap()
				else {
					panic!("save expected")
				};
				assert_eq!(changes.global_name, Some(Some(draft.name.clone())));
				state.apply(client_core::Envelope {
					generation: state.generation,
					event: client_core::Event::ProfileEdited {
						user,
						request,
						result: Err(client_core::auth::Failure::Ambiguous),
					},
				});
				let (_, labels) = frame(&ctx, &mut editor, &mut state, &mut avatars, width, vec![]);
				assert!(editor.draft.as_ref() == Some(&draft));
				assert!(!state.can_save_own_profile());
				let cancel = labels
					.iter()
					.find(|(text, _)| text == "Cancel")
					.unwrap()
					.1
					.center();
				for pressed in [true, false] {
					assert!(
						frame(
							&ctx,
							&mut editor,
							&mut state,
							&mut avatars,
							width,
							vec![
								egui::Event::PointerMoved(cancel),
								egui::Event::PointerButton {
									pos: cancel,
									button: egui::PointerButton::Primary,
									pressed,
									modifiers: egui::Modifiers::NONE
								}
							]
						)
						.0
						.is_empty()
					);
				}
				assert!(
					editor.draft.as_ref()
						== Some(&Draft::from_profile(
							state.own_profile.data.as_ref().unwrap()
						))
				);
			}
		}
	}
}
