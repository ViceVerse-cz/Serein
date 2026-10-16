//! Profile badges: the hover card, the account's customize menu (hide and reorder) and the
//! badge directory with tiers, rarity and progress. Layout follows the official client.
use crate::{
	avatars::Avatars,
	design,
	i18n::{translate, translate_args},
	icons::{self, Icon},
};
use egui::{Color32, CornerRadius, Pos2, Rect, RichText, Sense, Stroke, Vec2, pos2, vec2};
use model::{
	Id, ProfileBadge,
	badges::{BadgeCatalog, BadgeRarity, BadgeSettings, CatalogBadge, hideable, pinned},
};

const TILE: f32 = 48.0;
const DIRECTORY_TILE: f32 = 62.0;

/// Badge menus layered over an open profile card.
#[derive(Default)]
pub struct BadgePanels {
	customize: Option<Customize>,
	directory: Option<Directory>,
}
struct Customize {
	user: Id,
	anchor: Pos2,
	base: BadgeSettings,
	draft: BadgeSettings,
	/// `BadgeEditor::saved` when the draft was taken; a newer value means it was stored.
	saved: u64,
	/// `BadgeEditor::errors` when the menu opened or was reset; older errors stay hidden.
	errors: u64,
}
struct Directory {
	user: Id,
	selected: Option<u32>,
	levels: bool,
}

pub enum BadgeAction {
	Save(BadgeSettings),
}

impl BadgePanels {
	pub fn is_open(&self) -> bool {
		self.customize.is_some() || self.directory.is_some()
	}
	pub fn close(&mut self) {
		*self = Self::default();
	}
	/// Escape closes the menu before the directory beneath it.
	pub fn close_top(&mut self) {
		if self.customize.take().is_none() {
			self.directory = None;
		}
	}
	pub fn open_customize(
		&mut self,
		user: Id,
		anchor: Pos2,
		catalog: &BadgeCatalog,
		editor: &client_core::profile::BadgeEditor,
	) {
		if catalog.limited {
			self.customize = None;
			return;
		}
		let settings = catalog.settings();
		self.customize = Some(Customize {
			user,
			anchor,
			base: settings.clone(),
			draft: settings,
			saved: editor.saved,
			errors: editor.errors(),
		});
	}
	pub fn open_directory(&mut self, user: Id, selected: Option<u32>) {
		self.directory = Some(Directory {
			user,
			selected,
			levels: false,
		});
	}

	/// Shows open menus for `profile`; returns the rectangles they cover and a save request.
	pub fn show(
		&mut self,
		ctx: &egui::Context,
		profile: &model::UserProfile,
		own: bool,
		editor: &client_core::profile::BadgeEditor,
		avatars: &mut Avatars,
		demo: bool,
	) -> (Vec<Rect>, Option<BadgeAction>) {
		let mut covered = Vec::new();
		let mut action = None;
		let Some(catalog) = profile.badge_catalog.as_deref() else {
			self.close();
			return (covered, action);
		};
		if self
			.customize
			.as_ref()
			.is_some_and(|menu| menu.user != profile.user.id || !own || catalog.limited)
		{
			self.customize = None;
		}
		if self
			.directory
			.as_ref()
			.is_some_and(|directory| directory.user != profile.user.id)
		{
			self.directory = None;
		}
		let mut open_customize = None;
		if let Some(directory) = &mut self.directory {
			let (rect, close, customize) =
				directory_modal(ctx, directory, profile, catalog, own, avatars, demo);
			covered.push(rect);
			if close {
				self.directory = None;
			}
			open_customize = customize;
		}
		if let Some(anchor) = open_customize {
			// The menu replaces the directory rather than stacking above its modal layer.
			self.directory = None;
			self.open_customize(profile.user.id, anchor, catalog, editor);
		}
		if let Some(menu) = &mut self.customize {
			if editor.saved != menu.saved && !editor.saving {
				// The service confirmed a save and the catalog already holds it. Edits are
				// locked while saving, so nothing unsaved is lost.
				menu.base = catalog.settings();
				menu.draft = menu.base.clone();
				menu.saved = editor.saved;
			}
			let (rect, picked) = customize_menu(ctx, menu, catalog, editor, avatars, demo);
			covered.push(rect);
			match picked {
				Some(MenuAction::Save) => action = Some(BadgeAction::Save(menu.draft.clone())),
				Some(MenuAction::View) => {
					let selected = menu.draft.order.first().copied();
					self.customize = None;
					self.open_directory(profile.user.id, selected);
				}
				None => {}
			}
		}
		(covered, action)
	}
}

/// Badge row icon with Discord's hover card. Returns the response for click handling.
pub fn badge_icon(
	ui: &mut egui::Ui,
	avatars: &mut Avatars,
	badge: &ProfileBadge,
	catalog: Option<&BadgeCatalog>,
	size: f32,
	demo: bool,
) -> egui::Response {
	let entry = badge
		.number()
		.and_then(|number| catalog.and_then(|catalog| catalog.get(number)));
	let key = entry
		.and_then(CatalogBadge::icon)
		.map(art_key)
		.or_else(|| badge.icon_key());
	let mut response = avatars.show_icon(ui, key, size, demo, &badge.description);
	// Clicks open the catalog's menus, so without one the badge only hovers.
	if catalog.is_some() {
		response = response
			.interact(Sense::click())
			.on_hover_cursor(egui::CursorIcon::PointingHand);
	}
	response.on_hover_ui(|ui| hover_card(ui, avatars, badge, entry, demo))
}

fn art_key(url: &str) -> String {
	format!("badge-art:{url}")
}

fn hover_card(
	ui: &mut egui::Ui,
	avatars: &mut Avatars,
	badge: &ProfileBadge,
	entry: Option<&CatalogBadge>,
	demo: bool,
) {
	let colors = design::palette(ui);
	let Some(entry) = entry else {
		ui.label(&badge.description);
		return;
	};
	ui.set_max_width(220.0);
	ui.vertical_centered(|ui| {
		ui.spacing_mut().item_spacing.y = 2.0;
		avatars.show_icon(ui, entry.art().map(art_key), 72.0, demo, &entry.name);
		ui.add_space(4.0);
		let (eyebrow, title) = titles(entry);
		if let Some(eyebrow) = eyebrow {
			ui.label(RichText::new(eyebrow).size(12.0).color(colors.muted));
		}
		ui.label(design::semibold(ui, title.to_uppercase(), 15.0).color(colors.text_strong));
		if let Some(info) = entry.info.as_deref().or(Some(entry.description.as_str()))
			&& !info.is_empty()
		{
			ui.label(RichText::new(info).size(12.0).color(colors.muted));
		}
	});
}

/// Eyebrow and title, as the client words them: Nitro joins its tier name, an owned tiered
/// badge shows its tier beneath the badge name.
fn titles(entry: &CatalogBadge) -> (Option<String>, String) {
	let tier = entry
		.display_tier()
		.map(|tier| tier.name.clone())
		.filter(|name| !name.is_empty());
	if entry.id == model::badges::id::PREMIUM_TENURE {
		let name = translate("badges-nitro");
		return (
			None,
			tier.map_or(name.clone(), |tier| format!("{name} {tier}")),
		);
	}
	match tier {
		Some(tier) if entry.owned => (Some(entry.name.clone()), tier),
		_ => (None, entry.name.clone()),
	}
}

fn rarity_label(rarity: BadgeRarity) -> (String, Color32) {
	match rarity {
		BadgeRarity::Common => (
			translate("badges-rarity-common"),
			Color32::from_rgb(0x9a, 0x9d, 0xa6),
		),
		BadgeRarity::Rare => (
			translate("badges-rarity-rare"),
			Color32::from_rgb(0x4f, 0x8c, 0xff),
		),
		BadgeRarity::Epic => (
			translate("badges-rarity-epic"),
			Color32::from_rgb(0xa8, 0x5c, 0xf5),
		),
		BadgeRarity::Mythic => (
			translate("badges-rarity-mythic"),
			Color32::from_rgb(0xf0, 0xa2, 0x32),
		),
	}
}

fn rarity_pill(ui: &mut egui::Ui, rarity: BadgeRarity) {
	let (label, color) = rarity_label(rarity);
	let galley = ui.painter().layout_no_wrap(
		label.to_uppercase(),
		egui::FontId::new(11.0, design::semibold_family(ui.ctx())),
		color,
	);
	let size = vec2(galley.size().x + 32.0, galley.size().y + 6.0);
	let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
	let painter = ui.painter();
	painter.rect(
		rect,
		10,
		color.gamma_multiply(0.18),
		Stroke::new(1.0, color),
		egui::StrokeKind::Inside,
	);
	painter.circle_filled(pos2(rect.left() + 12.0, rect.center().y), 3.5, color);
	painter.galley(
		pos2(rect.left() + 21.0, rect.center().y - galley.size().y * 0.5),
		galley,
		color,
	);
}

fn date(seconds: i64) -> Option<String> {
	time::OffsetDateTime::from_unix_timestamp(seconds)
		.ok()
		.map(|date| crate::local_time::local(date).date().to_string())
}

enum MenuAction {
	Save,
	View,
}

/// Square badge tile; hidden badges are dimmed under an eye-slash.
fn tile(
	ui: &mut egui::Ui,
	avatars: &mut Avatars,
	entry: &CatalogBadge,
	size: f32,
	(hidden, selected): (bool, bool),
	sense: Sense,
	demo: bool,
) -> egui::Response {
	let colors = design::palette(ui);
	let (rect, response) = ui.allocate_exact_size(Vec2::splat(size), sense);
	let hovered = response.hovered();
	ui.painter().rect(
		rect,
		10,
		if hovered { colors.hover } else { colors.raised },
		Stroke::new(
			if selected { 1.5 } else { 1.0 },
			if selected {
				colors.accent
			} else {
				colors.border
			},
		),
		egui::StrokeKind::Inside,
	);
	let icon = Rect::from_center_size(rect.center(), Vec2::splat(size * 0.58));
	// A detached child keeps the icon from advancing the surrounding row.
	let mut child = ui.new_child(egui::UiBuilder::new().max_rect(icon));
	child.set_opacity(if hidden { 0.35 } else { 1.0 });
	avatars.show_icon(
		&mut child,
		entry.icon().map(art_key),
		icon.width(),
		demo,
		&entry.name,
	);
	if hidden {
		icons::paint(
			ui.painter(),
			Icon::EyeSlash,
			Rect::from_center_size(rect.center(), Vec2::splat(size * 0.5)),
			colors.text_strong,
		);
	}
	response.widget_info(|| {
		egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), &entry.name)
	});
	response
}

#[derive(Clone, Copy)]
struct Dragged(u32);

fn customize_menu(
	ctx: &egui::Context,
	menu: &mut Customize,
	catalog: &BadgeCatalog,
	editor: &client_core::profile::BadgeEditor,
	avatars: &mut Avatars,
	demo: bool,
) -> (Rect, Option<MenuAction>) {
	let mut picked = None;
	let area = egui::Area::new(egui::Id::unique(("badge-customize", menu.user)))
		.kind(egui::UiKind::Popup)
		.order(egui::Order::Foreground)
		.fixed_pos(menu.anchor)
		.constrain_to(ctx.content_rect().shrink(8.0))
		.show(ctx, |ui| {
			let colors = design::palette(ui);
			egui::Frame::new()
				.fill(colors.raised)
				.stroke(Stroke::new(1.0, colors.border))
				.corner_radius(12)
				.inner_margin(16)
				.show(ui, |ui| {
					ui.set_width(344.0);
					ui.spacing_mut().item_spacing = vec2(8.0, 8.0);
					ui.horizontal(|ui| {
						icons::inline(ui, Icon::Sparkle, 16.0, colors.text_strong);
						ui.label(
							design::semibold(ui, translate("badges-customize-title"), 16.0)
								.color(colors.text_strong),
						);
					});
					ui.label(
						RichText::new(translate("badges-customize-hint"))
							.size(12.0)
							.color(colors.muted),
					);
					let pinned_badges: Vec<&CatalogBadge> =
						catalog.owned().filter(|entry| pinned(entry.id)).collect();
					let ordered: Vec<(u32, bool)> = menu
						.draft
						.order
						.iter()
						.map(|id| (*id, false))
						.chain(menu.draft.hidden.iter().map(|id| (*id, true)))
						.collect();
					let mut toggle = None;
					let mut drop: Option<(u32, usize)> = None;
					ui.horizontal_wrapped(|ui| {
						ui.spacing_mut().item_spacing = vec2(8.0, 8.0);
						for entry in &pinned_badges {
							tile(
								ui,
								avatars,
								entry,
								TILE,
								(false, false),
								Sense::hover(),
								demo,
							)
							.on_hover_text(translate("badges-always-visible"));
						}
						for (index, (id, hidden)) in ordered.iter().enumerate() {
							let Some(entry) = catalog.get(*id) else {
								continue;
							};
							let response = tile(
								ui,
								avatars,
								entry,
								TILE,
								(*hidden, false),
								Sense::click_and_drag(),
								demo,
							);
							if !hidden {
								response.dnd_set_drag_payload(Dragged(*id));
							}
							if let Some(payload) = response.dnd_release_payload::<Dragged>()
								&& payload.0 != *id
							{
								drop = Some((payload.0, index.min(menu.draft.order.len())));
							}
							if response.dnd_hover_payload::<Dragged>().is_some() {
								ui.painter().rect_stroke(
									response.rect.expand(2.0),
									12,
									Stroke::new(2.0, colors.accent),
									egui::StrokeKind::Outside,
								);
							}
							let hint = if !hideable(*id) {
								translate("badges-always-visible")
							} else if *hidden {
								translate("badges-click-show")
							} else {
								translate("badges-click-hide")
							};
							let response =
								response.on_hover_text(format!("{}\n{hint}", entry.name));
							if response.clicked() && hideable(*id) {
								toggle = Some((*id, !hidden));
							}
						}
					});
					// The draft stays as sent until the service answers.
					if let Some((id, hidden)) = toggle.filter(|_| !editor.saving) {
						menu.draft.set_hidden(id, hidden);
					}
					if let Some((id, to)) = drop.filter(|_| !editor.saving) {
						menu.draft.set_hidden(id, false);
						menu.draft.reorder(id, to);
					}
					if pinned_badges.is_empty() && ordered.is_empty() {
						ui.label(
							RichText::new(translate("badges-none"))
								.size(13.0)
								.color(colors.muted),
						);
					}
					if let Some(error) = editor.error_since(menu.errors) {
						ui.label(RichText::new(error).size(12.0).color(colors.danger));
					}
					let changed = menu.draft != menu.base;
					if changed || editor.saving {
						ui.columns(2, |columns| {
							if design::secondary_button(&mut columns[0], &translate("badges-reset"))
								.clicked() && !editor.saving
							{
								menu.draft = menu.base.clone();
								menu.errors = editor.errors();
							}
							let label = if editor.saving {
								translate("badges-saving")
							} else {
								translate("badges-save")
							};
							if columns[1]
								.add_enabled_ui(!editor.saving && changed, |ui| {
									design::primary_button(ui, &label)
								})
								.inner
								.clicked()
							{
								picked = Some(MenuAction::Save);
							}
						});
					}
					if design::secondary_button(ui, &translate("badges-view")).clicked() {
						picked = Some(MenuAction::View);
					}
				});
		});
	ctx.move_to_top(area.response.layer_id);
	(area.response.rect, picked)
}

/// Returns the modal rectangle, whether it closed, and where to open the customize menu.
fn directory_modal(
	ctx: &egui::Context,
	directory: &mut Directory,
	profile: &model::UserProfile,
	catalog: &BadgeCatalog,
	own: bool,
	avatars: &mut Avatars,
	demo: bool,
) -> (Rect, bool, Option<Pos2>) {
	let mut close = false;
	let mut customize = None;
	let owned: Vec<&CatalogBadge> = catalog.owned().collect();
	let earnable: Vec<&CatalogBadge> = if own {
		catalog.earnable().collect()
	} else {
		Vec::new()
	};
	if directory
		.selected
		.is_none_or(|id| !owned.iter().chain(&earnable).any(|entry| entry.id == id))
	{
		directory.selected = owned.first().or(earnable.first()).map(|entry| entry.id);
	}
	let width = (ctx.content_rect().width() - 48.0).clamp(1.0, 740.0);
	let height = (ctx.content_rect().height() - 48.0).clamp(1.0, 520.0);
	let modal = egui::Modal::new(egui::Id::unique(("badge-directory", directory.user)))
		.backdrop_color(crate::dialog::backdrop(ctx))
		.frame(
			egui::Frame::new()
				.fill(design::palette_for(ctx).sidebar)
				.corner_radius(12),
		)
		.show(ctx, |ui| {
			ui.set_width(width);
			let split = width >= 640.0;
			let list_width = if split { 320.0 } else { width };
			let view = DirectoryView {
				profile,
				owned: &owned,
				earnable: &earnable,
				own,
				complete: !catalog.limited,
				width: list_width,
			};
			if split {
				ui.horizontal_top(|ui| {
					ui.spacing_mut().item_spacing.x = 0.0;
					ui.allocate_ui_with_layout(
						vec2(list_width, height),
						egui::Layout::top_down(egui::Align::Min),
						|ui| {
							egui::ScrollArea::vertical()
								.id_salt("badge-directory-list")
								.max_height(height)
								.auto_shrink([false, false])
								.show(ui, |ui| {
									customize = directory_list(ui, directory, &view, avatars, demo);
								});
						},
					);
					ui.allocate_ui_with_layout(
						vec2(width - list_width, height),
						egui::Layout::top_down(egui::Align::Min),
						|ui| {
							egui::ScrollArea::vertical()
								.id_salt("badge-directory-details")
								.max_height(height)
								.auto_shrink([false, false])
								.show(ui, |ui| {
									directory_details(ui, directory, catalog, avatars, demo)
								});
						},
					);
				});
			} else {
				egui::ScrollArea::vertical()
					.id_salt("badge-directory-stacked")
					.max_height(height)
					.auto_shrink([false, false])
					.show(ui, |ui| {
						customize = directory_list(ui, directory, &view, avatars, demo);
						directory_details(ui, directory, catalog, avatars, demo);
					});
			}
			let corner = ui.min_rect().right_top() + vec2(-40.0, 12.0);
			ui.scope_builder(
				egui::UiBuilder::new().max_rect(Rect::from_min_size(corner, Vec2::splat(28.0))),
				|ui| {
					if icons::button(ui, Icon::Close, 28.0, &translate("badges-close")).clicked() {
						close = true;
					}
				},
			);
		});
	if modal.should_close() {
		close = true;
	}
	(modal.response.rect, close, customize)
}

struct DirectoryView<'a> {
	profile: &'a model::UserProfile,
	owned: &'a [&'a CatalogBadge],
	earnable: &'a [&'a CatalogBadge],
	own: bool,
	complete: bool,
	width: f32,
}

/// Badge grids; returns where to open the customize menu when its button was pressed.
fn directory_list(
	ui: &mut egui::Ui,
	directory: &mut Directory,
	view: &DirectoryView,
	avatars: &mut Avatars,
	demo: bool,
) -> Option<Pos2> {
	let colors = design::palette(ui);
	let mut customize = None;
	egui::Frame::new().inner_margin(24).show(ui, |ui| {
		ui.set_width(view.width - 48.0);
		ui.spacing_mut().item_spacing = vec2(8.0, 8.0);
		let title = if view.own {
			translate("badges-your")
		} else {
			translate_args("badges-user", &[("name", view.profile.user.name.as_str())])
		};
		ui.label(design::semibold(ui, &title, 20.0).color(colors.text_strong));
		if view.own {
			ui.label(
				RichText::new(translate("badges-browse"))
					.size(13.0)
					.color(colors.muted),
			);
		}
		ui.add_space(8.0);
		let mut grid = |ui: &mut egui::Ui, entries: &[&CatalogBadge]| {
			ui.horizontal_wrapped(|ui| {
				ui.spacing_mut().item_spacing = vec2(8.0, 8.0);
				for entry in entries {
					let selected = directory.selected == Some(entry.id);
					if tile(
						ui,
						avatars,
						entry,
						DIRECTORY_TILE,
						(false, selected),
						Sense::click(),
						demo,
					)
					.on_hover_text(&entry.name)
					.clicked()
					{
						directory.selected = Some(entry.id);
						directory.levels = false;
					}
				}
			});
		};
		if view.owned.is_empty() {
			ui.label(
				RichText::new(translate("badges-none"))
					.size(13.0)
					.color(colors.muted),
			);
		}
		grid(ui, view.owned);
		if !view.earnable.is_empty() {
			ui.add_space(8.0);
			ui.label(
				design::semibold(ui, translate("badges-available"), 14.0).color(colors.text_strong),
			);
			grid(ui, view.earnable);
		}
		if view.own && view.complete && !view.owned.is_empty() {
			ui.add_space(12.0);
			let button = design::secondary_button(ui, &translate("badges-customize-button"));
			if button.clicked() {
				customize = Some(button.rect.left_bottom() + vec2(0.0, 8.0));
			}
		}
	});
	customize
}

fn directory_details(
	ui: &mut egui::Ui,
	directory: &mut Directory,
	catalog: &BadgeCatalog,
	avatars: &mut Avatars,
	demo: bool,
) {
	let Some(entry) = directory.selected.and_then(|id| catalog.get(id)) else {
		return;
	};
	egui::Frame::new()
		.fill(design::palette(ui).chat)
		.corner_radius(CornerRadius {
			nw: 0,
			ne: 12,
			sw: 0,
			se: 12,
		})
		.inner_margin(24)
		.show(ui, |ui| {
			ui.set_width(ui.available_width());
			badge_details(ui, avatars, entry, &mut directory.levels, demo);
		});
}

fn badge_details(
	ui: &mut egui::Ui,
	avatars: &mut Avatars,
	entry: &CatalogBadge,
	levels: &mut bool,
	demo: bool,
) {
	let colors = design::palette(ui);
	ui.spacing_mut().item_spacing.y = 6.0;
	ui.vertical_centered(|ui| {
		ui.add_space(12.0);
		avatars.show_icon(ui, entry.art().map(art_key), 112.0, demo, &entry.name);
		ui.add_space(6.0);
		let (eyebrow, title) = titles(entry);
		if let Some(eyebrow) = eyebrow {
			ui.label(RichText::new(eyebrow).size(13.0).color(colors.muted));
		}
		ui.label(design::semibold(ui, title.to_uppercase(), 22.0).color(colors.text_strong));
		if let Some(info) = &entry.info {
			ui.label(RichText::new(info).size(13.0).color(colors.muted));
		}
	});
	ui.add_space(10.0);
	let card = |ui: &mut egui::Ui, content: &mut dyn FnMut(&mut egui::Ui)| {
		egui::Frame::new()
			.fill(colors.raised)
			.corner_radius(10)
			.inner_margin(12)
			.show(ui, |ui| {
				ui.set_width(ui.available_width());
				ui.set_min_height(40.0);
				content(ui);
			});
	};
	ui.columns(2, |columns| {
		card(&mut columns[0], &mut |ui| {
			ui.vertical_centered(|ui| {
				let unlocked = entry
					.owned
					.then(|| entry.unlocked_at().and_then(date))
					.flatten();
				let value = if entry.owned {
					unlocked.unwrap_or_else(|| "—".into())
				} else {
					translate("badges-locked")
				};
				ui.label(RichText::new(value).size(14.0).color(colors.text_strong));
				ui.label(
					RichText::new(translate("badges-date-unlocked"))
						.size(12.0)
						.color(colors.muted),
				);
			});
		});
		card(&mut columns[1], &mut |ui| {
			ui.vertical_centered(|ui| {
				match entry.rarity() {
					Some(rarity) => rarity_pill(ui, rarity),
					None => {
						ui.label(RichText::new("—").size(14.0).color(colors.text_strong));
					}
				}
				ui.label(
					RichText::new(translate("badges-rarity"))
						.size(12.0)
						.color(colors.muted),
				);
			});
		});
	});
	if entry.tiers.is_empty() && entry.description.is_empty() {
		return;
	}
	card(ui, &mut |ui| {
		ui.spacing_mut().item_spacing.y = 8.0;
		let maxed = entry.owned && !entry.tiers.is_empty() && entry.next_tier.is_none();
		let text = if maxed {
			translate("badges-max-level")
		} else {
			entry.description.clone()
		};
		if !text.is_empty() {
			ui.label(RichText::new(text).size(13.0).color(colors.text));
		}
		if let Some(next) = entry.next() {
			ui.horizontal(|ui| {
				ui.spacing_mut().item_spacing.x = 10.0;
				let current = entry.current().and_then(|tier| tier.icon.as_deref());
				avatars.show_icon(
					ui,
					current.or(entry.icon.as_deref()).map(art_key),
					32.0,
					demo,
					"",
				);
				let bar_width = (ui.available_width() - 52.0).max(40.0);
				ui.vertical(|ui| {
					ui.set_width(bar_width);
					ui.spacing_mut().item_spacing.y = 4.0;
					ui.label(
						design::semibold(
							ui,
							translate_args("badges-next-level", &[("tier", next.name.as_str())]),
							14.0,
						)
						.color(colors.text_strong),
					);
					let fraction = entry.progress.as_ref().map_or(0.0, |p| p.fraction());
					let (bar, _) = ui.allocate_exact_size(vec2(bar_width, 6.0), Sense::hover());
					ui.painter().rect_filled(bar, 3, colors.hover);
					ui.painter().rect_filled(
						Rect::from_min_size(bar.min, vec2(bar.width() * fraction, bar.height())),
						3,
						colors.accent,
					);
					if let Some(helper) = entry
						.progress
						.as_ref()
						.map(|p| p.helper.as_str())
						.filter(|helper| !helper.is_empty())
					{
						ui.label(RichText::new(helper).size(12.0).color(colors.muted));
					}
				});
				avatars.show_icon(
					ui,
					next.icon.as_deref().map(art_key),
					32.0,
					demo,
					&next.name,
				);
			});
		}
		if entry.tiers.len() > 1 {
			ui.vertical_centered(|ui| {
				let label = if *levels {
					translate("badges-hide-levels")
				} else {
					translate("badges-all-levels")
				};
				let response = ui.add(
					egui::Button::new(RichText::new(label).size(12.0).color(colors.muted))
						.frame(false),
				);
				if response.clicked() {
					*levels = !*levels;
				}
			});
			if *levels {
				let reached = entry
					.current_tier
					.as_deref()
					.and_then(|key| entry.tiers.iter().position(|tier| tier.key == key));
				for (index, tier) in entry.tiers.iter().enumerate() {
					ui.horizontal(|ui| {
						ui.spacing_mut().item_spacing.x = 10.0;
						avatars.show_icon(
							ui,
							tier.icon.as_deref().map(art_key),
							28.0,
							demo,
							&tier.name,
						);
						ui.vertical(|ui| {
							ui.spacing_mut().item_spacing.y = 0.0;
							ui.label(
								design::semibold(ui, &tier.name, 13.0).color(colors.text_strong),
							);
							if !tier.milestone.is_empty() {
								ui.label(
									RichText::new(&tier.milestone)
										.size(12.0)
										.color(colors.muted),
								);
							}
						});
						if reached.is_some_and(|reached| index <= reached) {
							ui.with_layout(
								egui::Layout::right_to_left(egui::Align::Center),
								|ui| {
									icons::inline(ui, Icon::Check, 14.0, colors.positive);
								},
							);
						}
					});
				}
			}
		}
	});
}

/// Synthetic catalog for offline previews: artwork is drawn locally, never downloaded.
#[cfg(any(test, feature = "demo"))]
pub fn synthetic_catalog() -> BadgeCatalog {
	use model::badges::{BadgeProgress, BadgeTier, id};
	let url = |name: &str| {
		Some(format!(
			"https://cdn.discordapp.com/synthetic/badges/{name}.png"
		))
	};
	let tier = |key: &str, name: &str, milestone: &str, threshold: f64, rarity| BadgeTier {
		key: key.into(),
		name: name.into(),
		icon: url(key),
		art: url(&format!("{key}-art")),
		rarity: Some(rarity),
		milestone: milestone.into(),
		threshold: Some(threshold),
		obtained_at: None,
	};
	let badge = |id: u32, name: &str| CatalogBadge {
		id,
		name: name.into(),
		description: String::new(),
		owned: true,
		hidden: false,
		earnable: true,
		icon: url(&format!("badge-{id}")),
		art: None,
		rarity: Some(BadgeRarity::Common),
		tiers: vec![],
		current_tier: None,
		next_tier: None,
		progress: None,
		obtained_at: Some(1_776_556_800),
		info: None,
	};
	BadgeCatalog {
		badges: vec![
			CatalogBadge {
				description: "Stay subscribed to Nitro to level up this badge.".into(),
				info: Some("Subscriber since 2026-04-19".into()),
				tiers: vec![
					tier("starter", "Starter", "1 month", 0.0, BadgeRarity::Common),
					tier("bronze", "Bronze", "3 months", 3.0, BadgeRarity::Common),
					tier("silver", "Silver", "6 months", 6.0, BadgeRarity::Rare),
				],
				current_tier: Some("starter".into()),
				next_tier: Some("bronze".into()),
				progress: Some(BadgeProgress {
					current: 2.4,
					threshold: 3.0,
					helper: "About 3 weeks to go".into(),
				}),
				..badge(id::PREMIUM_TENURE, "Nitro")
			},
			CatalogBadge {
				description: "Celebrates how long this account has been on Discord.".into(),
				info: Some("8 years, 1 month".into()),
				tiers: vec![
					tier("sequoia", "Sequoia", "7 years", 7.0, BadgeRarity::Rare),
					tier(
						"bristlecone",
						"Bristlecone",
						"8 years",
						8.0,
						BadgeRarity::Epic,
					),
					tier(
						"stromatolite",
						"Stromatolite",
						"9 years",
						9.0,
						BadgeRarity::Epic,
					),
					tier(
						"primordial",
						"Primordial",
						"10+ years",
						10.0,
						BadgeRarity::Mythic,
					),
				],
				current_tier: Some("bristlecone".into()),
				next_tier: Some("stromatolite".into()),
				progress: Some(BadgeProgress {
					current: 8.1,
					threshold: 9.0,
					helper: "11 months to go".into(),
				}),
				..badge(id::ACCOUNT_AGE, "Account Age")
			},
			CatalogBadge {
				hidden: true,
				info: Some("120 hours streamed".into()),
				..badge(id::STREAMING, "Streaming")
			},
			CatalogBadge {
				owned: false,
				description: "Play games with Discord open to unlock this badge.".into(),
				progress: Some(BadgeProgress {
					current: 6.0,
					threshold: 10.0,
					helper: "4 more hours".into(),
				}),
				..badge(id::GAME_TIME, "Game Time")
			},
		],
		limited: false,
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn synthetic_catalog_is_a_valid_profile_catalog() {
		assert!(synthetic_catalog().valid());
	}

	#[test]
	fn titles_follow_the_official_wording() {
		let catalog = synthetic_catalog();
		let (eyebrow, title) = titles(catalog.get(model::badges::id::ACCOUNT_AGE).unwrap());
		assert_eq!(
			(eyebrow.as_deref(), title.as_str()),
			(Some("Account Age"), "Bristlecone")
		);
		let (eyebrow, title) = titles(catalog.get(model::badges::id::PREMIUM_TENURE).unwrap());
		assert!(eyebrow.is_none() && title.ends_with("Starter"));
		let (eyebrow, title) = titles(catalog.get(model::badges::id::GAME_TIME).unwrap());
		assert_eq!((eyebrow, title.as_str()), (None, "Game Time"));
	}

	#[test]
	fn incomplete_catalogs_cannot_open_or_keep_a_customize_draft() {
		let ctx = egui::Context::default();
		let mut panels = BadgePanels::default();
		let mut avatars = Avatars::default();
		let mut profile =
			crate::profiles::synthetic(&test_support::message(1, Id(20)).author, None);
		let mut catalog = synthetic_catalog();
		let editor = client_core::profile::BadgeEditor::default();
		catalog.limited = true;
		panels.open_customize(profile.user.id, pos2(10.0, 10.0), &catalog, &editor);
		assert!(panels.customize.is_none());
		catalog.limited = false;
		panels.open_customize(profile.user.id, pos2(10.0, 10.0), &catalog, &editor);
		panels
			.customize
			.as_mut()
			.unwrap()
			.draft
			.set_hidden(model::badges::id::STREAMING, false);
		catalog.limited = true;
		profile.badge_catalog = Some(Box::new(catalog));
		ctx.run_ui(egui::RawInput::default(), |ui| {
			let (_, action) = panels.show(ui.ctx(), &profile, true, &editor, &mut avatars, true);
			assert!(action.is_none());
		})
		.drop_without_applying_deltas();
		assert!(panels.customize.is_none());
		panels.open_directory(profile.user.id, None);
		let output = ctx.run_ui(egui::RawInput::default(), |ui| {
			panels.show(ui.ctx(), &profile, true, &editor, &mut avatars, true);
		});
		assert!(!output.shapes.iter().any(|shape| matches!(&shape.shape,
			egui::Shape::Text(text) if text.galley.job.text == translate("badges-customize-button"))));
		output.drop_without_applying_deltas();
	}

	#[test]
	fn directory_fits_short_windows_and_scrolls_each_wide_column() {
		for size in [vec2(1100.0, 700.0), vec2(1100.0, 360.0), vec2(600.0, 360.0)] {
			let ctx = egui::Context::default();
			let mut avatars = Avatars::default();
			let mut profile =
				crate::profiles::synthetic(&test_support::message(1, Id(20)).author, None);
			let mut catalog = synthetic_catalog();
			let template = catalog.badges[2].clone();
			for id in 30..66 {
				catalog.badges.push(CatalogBadge {
					id,
					..template.clone()
				});
			}
			let tier = catalog.badges[0].tiers[0].clone();
			catalog.badges[0].tiers = (0..model::badges::MAX_BADGE_TIERS)
				.map(|index| model::badges::BadgeTier {
					key: format!("tier-{index}"),
					name: format!("Tier {index}"),
					..tier.clone()
				})
				.collect();
			catalog.badges[0].current_tier = Some("tier-0".into());
			catalog.badges[0].next_tier = Some("tier-1".into());
			let details_title = titles(&catalog.badges[0]).1.to_uppercase();
			assert!(catalog.valid());
			profile.badge_catalog = Some(Box::new(catalog));
			let screen = Rect::from_min_size(Pos2::ZERO, size);
			let mut directory = Directory {
				user: profile.user.id,
				selected: Some(model::badges::id::PREMIUM_TENURE),
				levels: true,
			};
			let mut frame = |pointer, scroll| {
				let mut rect = Rect::NOTHING;
				let output = ctx.run_ui(
					egui::RawInput {
						screen_rect: Some(screen),
						events: vec![egui::Event::PointerMoved(pointer)],
						..Default::default()
					},
					|ui| {
						ui.input_mut(|input| input.smooth_scroll_delta.y = scroll);
						rect = directory_modal(
							ui.ctx(),
							&mut directory,
							&profile,
							profile.badge_catalog.as_deref().unwrap(),
							true,
							&mut avatars,
							true,
						)
						.0;
					},
				);
				(rect, output)
			};
			for _ in 0..3 {
				frame(Pos2::ZERO, 0.0).1.drop_without_applying_deltas();
			}
			let (rect, output) = frame(Pos2::ZERO, 0.0);
			assert!(screen.contains_rect(rect), "{size:?}: {rect:?}");
			let text_y = |output: &egui::FullOutput, label: &str| {
				output
					.shapes
					.iter()
					.find_map(|shape| match &shape.shape {
						egui::Shape::Text(text) if text.galley.job.text == label => {
							Some(text.pos.y)
						}
						_ => None,
					})
					.unwrap_or_else(|| panic!("Missing label {label}"))
			};
			if size.x > 1000.0 {
				let list_title = translate("badges-your");
				let original_list = text_y(&output, &list_title);
				let original_details = text_y(&output, &details_title);
				output.drop_without_applying_deltas();
				let pointer = rect.left_top() + vec2(100.0, 100.0);
				frame(pointer, 0.0).1.drop_without_applying_deltas();
				frame(pointer, -10.0).1.drop_without_applying_deltas();
				let (_, output) = frame(pointer, 0.0);
				let scrolled_list = text_y(&output, &list_title);
				assert!(scrolled_list < original_list - 1.0);
				assert!((text_y(&output, &details_title) - original_details).abs() < 0.1);
				output.drop_without_applying_deltas();
				let pointer = rect.right_top() + vec2(-100.0, 100.0);
				frame(pointer, 0.0).1.drop_without_applying_deltas();
				frame(pointer, -10.0).1.drop_without_applying_deltas();
				let (_, output) = frame(pointer, 0.0);
				assert!(text_y(&output, &details_title) < original_details - 1.0);
				assert!((text_y(&output, &list_title) - scrolled_list).abs() < 0.1);
				output.drop_without_applying_deltas();
			} else {
				output.drop_without_applying_deltas();
			}
		}
	}

	#[test]
	fn saving_locks_badge_edits_and_reopened_menu_receives_confirmation() {
		let ctx = egui::Context::default();
		let mut panels = BadgePanels::default();
		let mut avatars = Avatars::default();
		let mut profile =
			crate::profiles::synthetic(&test_support::message(1, Id(20)).author, None);
		profile.badge_catalog = Some(Box::new(synthetic_catalog()));
		let mut editor = client_core::profile::BadgeEditor::default();
		editor.saving = true;
		panels.open_customize(
			profile.user.id,
			pos2(10.0, 10.0),
			profile.badge_catalog.as_deref().unwrap(),
			&editor,
		);
		let mut frame = |events| {
			ctx.run_ui(
				egui::RawInput {
					events,
					..Default::default()
				},
				|ui| {
					let (_, action) =
						panels.show(ui.ctx(), &profile, true, &editor, &mut avatars, true);
					assert!(action.is_none());
				},
			)
		};
		frame(vec![]).drop_without_applying_deltas();
		let output = frame(vec![]);
		let tile = output
			.shapes
			.iter()
			.filter_map(|shape| match &shape.shape {
				egui::Shape::Rect(rect) if rect.rect.size() == Vec2::splat(TILE) => Some(rect.rect),
				_ => None,
			})
			.next_back()
			.expect("The hidden Streaming badge is drawn");
		output.drop_without_applying_deltas();
		frame(vec![
			egui::Event::PointerMoved(tile.center()),
			egui::Event::PointerButton {
				pos: tile.center(),
				button: egui::PointerButton::Primary,
				pressed: true,
				modifiers: egui::Modifiers::NONE,
			},
		])
		.drop_without_applying_deltas();
		frame(vec![egui::Event::PointerButton {
			pos: tile.center(),
			button: egui::PointerButton::Primary,
			pressed: false,
			modifiers: egui::Modifiers::NONE,
		}])
		.drop_without_applying_deltas();
		assert_eq!(
			panels.customize.as_ref().unwrap().draft.hidden,
			[model::badges::id::STREAMING]
		);
		panels.close();
		panels.open_customize(
			profile.user.id,
			pos2(10.0, 10.0),
			profile.badge_catalog.as_deref().unwrap(),
			&editor,
		);
		let catalog = profile.badge_catalog.as_deref_mut().unwrap();
		let mut accepted = catalog.settings();
		accepted.set_hidden(model::badges::id::STREAMING, false);
		catalog.apply(&accepted);
		editor.saving = false;
		editor.saved = 1;
		ctx.run_ui(egui::RawInput::default(), |ui| {
			panels.show(ui.ctx(), &profile, true, &editor, &mut avatars, true);
		})
		.drop_without_applying_deltas();
		let menu = panels.customize.as_ref().unwrap();
		assert_eq!(menu.base, accepted);
		assert_eq!(menu.draft, accepted);
	}

	#[test]
	fn panels_save_only_changed_drafts_and_rebase_after_confirmation() {
		let ctx = egui::Context::default();
		let mut panels = BadgePanels::default();
		let mut avatars = Avatars::default();
		let mut profile =
			crate::profiles::synthetic(&test_support::message(1, Id(20)).author, None);
		profile.badge_catalog = Some(Box::new(synthetic_catalog()));
		let catalog = profile.badge_catalog.as_deref().unwrap().clone();
		let mut editor = client_core::profile::BadgeEditor::default();
		panels.open_customize(profile.user.id, pos2(10.0, 10.0), &catalog, &editor);
		let menu = panels.customize.as_mut().unwrap();
		assert_eq!(menu.draft.hidden, [model::badges::id::STREAMING]);
		menu.draft.set_hidden(model::badges::id::STREAMING, false);
		assert_ne!(menu.draft, menu.base);
		editor.saved = 1;
		ctx.run_ui(egui::RawInput::default(), |ui| {
			panels.show(ui.ctx(), &profile, true, &editor, &mut avatars, true);
		})
		.drop_without_applying_deltas();
		// The confirmed baseline comes from the catalog, not from the draft.
		let menu = panels.customize.as_ref().unwrap();
		assert_eq!(menu.draft, menu.base);
		assert_eq!(menu.base, catalog.settings());
		// Another user's profile never keeps the account's customize menu open.
		profile.user.id = Id(77);
		ctx.run_ui(egui::RawInput::default(), |ui| {
			panels.show(ui.ctx(), &profile, false, &editor, &mut avatars, true);
		})
		.drop_without_applying_deltas();
		assert!(panels.customize.is_none());
		panels.open_directory(profile.user.id, None);
		panels.close_top();
		assert!(!panels.is_open());
	}
}
