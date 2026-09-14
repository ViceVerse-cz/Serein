use crate::design::LazyHover;
use crate::{
	MessagingUi, design,
	icons::{self, Icon},
	notifications::{badge, rail_indicator},
};
use client_core::{Command, State};
use egui::{Color32, Sense};
use model::{
	Id,
	guild_folders::{Folder, Settings},
};
use std::collections::BTreeSet;

#[derive(Default)]
pub(super) struct FolderUi {
	expanded: BTreeSet<u64>,
	editor: Option<(u64, String, [u8; 3])>,
	generation: u64,
	key: Option<(u64, u64)>,
	// Fixed-size rows: bounded to (MAX_NAV + MAX_FOLDERS) * size_of::<Row>() bytes.
	rows: Box<[Row]>,
}
#[derive(Clone, Copy, Debug, PartialEq)]
enum Item {
	Server(Id),
	Folder(u64),
}
type Row = (Item, Option<(u64, u32)>);

impl FolderUi {
	fn sync_rows(&mut self, state: &State) -> bool {
		let key = (state.generation, state.revision);
		if self.key == Some(key) {
			return false;
		}
		let mut rows = Vec::new();
		let mut seen = BTreeSet::new();
		if let Some(settings) = &state.guild_folders {
			self.expanded
				.retain(|id| settings.folders.iter().any(|f| f.id == Some(*id)));
			for folder in &settings.folders {
				if let Some(id) = folder.id {
					rows.push((
						Item::Folder(id),
						self.expanded
							.contains(&id)
							.then_some((id, folder.color.unwrap_or(design::DEFAULT_PRIMARY_RGB))),
					));
				}
				for &id in &folder.guild_ids {
					seen.insert(id);
					if folder.id.is_none_or(|id| self.expanded.contains(&id))
						&& state.guild(id).is_some()
					{
						rows.push((
							Item::Server(id),
							folder.id.map(|folder_id| {
								(
									folder_id,
									folder.color.unwrap_or(design::DEFAULT_PRIMARY_RGB),
								)
							}),
						));
					}
				}
			}
		}
		rows.extend(
			state
				.guilds
				.iter()
				.filter(|g| !seen.contains(&g.id))
				.map(|g| (Item::Server(g.id), None)),
		);
		// Validated settings contain each guild only once, plus at most MAX_FOLDERS headers.
		self.rows = rows
			.into_iter()
			.take(client_core::MAX_NAV + model::guild_folders::MAX_FOLDERS)
			.collect();
		self.key = Some(key);
		true
	}
	fn toggle(&mut self, id: u64) {
		if !self.expanded.remove(&id) {
			self.expanded.insert(id);
		}
		self.key = None;
	}
}
#[derive(Clone, Copy, Debug, PartialEq)]
enum Placement {
	Before,
	Inside,
	After,
}
enum Edit {
	Drop(Item, Item, Placement),
	Outside(Id),
	Shift(Item, bool),
	Dissolve(u64),
	Customize(u64, String, u32),
}

fn entry(settings: &Settings, item: Item) -> Option<usize> {
	settings.folders.iter().position(|f| match item {
		Item::Folder(id) => f.id == Some(id),
		Item::Server(id) => f.guild_ids.contains(&id),
	})
}
fn standalone(id: Id) -> Folder {
	Folder {
		id: None,
		guild_ids: vec![id],
		name: None,
		color: None,
	}
}
fn edit(settings: &mut Settings, edit: Edit) {
	match edit {
		Edit::Customize(id, name, color) => {
			if let Some(f) = settings.folders.iter_mut().find(|f| f.id == Some(id)) {
				f.name = (!name.is_empty()).then_some(name);
				f.color = Some(color);
			}
		}
		Edit::Dissolve(id) => {
			if let Some(i) = entry(settings, Item::Folder(id)) {
				let folder = settings.folders.remove(i);
				settings
					.folders
					.splice(i..i, folder.guild_ids.into_iter().map(standalone));
			}
		}
		Edit::Outside(id) => {
			for f in &mut settings.folders {
				f.guild_ids.retain(|g| *g != id);
			}
			settings.folders.push(standalone(id));
		}
		Edit::Shift(item, down) => {
			if let Some(i) = entry(settings, item) {
				// Servers inside a folder reorder within that folder.
				if let Item::Server(id) = item
					&& settings.folders[i].id.is_some()
				{
					let ids = &mut settings.folders[i].guild_ids;
					let j = ids.iter().position(|g| *g == id).unwrap();
					let k = if down {
						(j + 1).min(ids.len() - 1)
					} else {
						j.saturating_sub(1)
					};
					ids.swap(j, k);
				} else {
					let j = if down {
						(i + 1).min(settings.folders.len() - 1)
					} else {
						i.saturating_sub(1)
					};
					settings.folders.swap(i, j);
				}
			}
		}
		Edit::Drop(source, target, placement) => {
			if source == target {
				return;
			}
			let Some(source_index) = entry(settings, source) else {
				return;
			};
			let Some(target_index) = entry(settings, target) else {
				return;
			};
			if let Item::Folder(_) = source {
				if source_index != target_index {
					let moved = settings.folders.remove(source_index);
					let index = entry(settings, target).unwrap();
					settings
						.folders
						.insert(index + usize::from(placement == Placement::After), moved);
				}
			} else if let Item::Server(id) = source {
				for folder in &mut settings.folders {
					folder.guild_ids.retain(|g| *g != id);
				}
				match placement {
					Placement::Inside => {
						let next_id = (1..=u32::MAX as u64)
							.find(|id| !settings.folders.iter().any(|f| f.id == Some(*id)))
							.unwrap();
						let folder = &mut settings.folders[target_index];
						if folder.id.is_none() {
							folder.id = Some(next_id);
							folder.color = Some(design::DEFAULT_PRIMARY_RGB);
						}
						folder.guild_ids.push(id);
					}
					Placement::Before | Placement::After => {
						let after = usize::from(placement == Placement::After);
						let folder = &mut settings.folders[target_index];
						if let Item::Server(target) = target
							&& folder.id.is_some()
						{
							let index = folder.guild_ids.iter().position(|g| *g == target).unwrap();
							folder.guild_ids.insert(index + after, id);
						} else {
							settings
								.folders
								.insert(target_index + after, standalone(id));
						}
					}
				}
			}
		}
	}
	settings.folders.retain(|f| !f.guild_ids.is_empty());
}

fn drop_target(
	rows: &[(Item, egui::Rect)],
	source: Item,
	y: f32,
) -> Option<(Item, egui::Rect, Placement)> {
	let &(item, rect) = rows.iter().min_by(|a, b| {
		(a.1.center().y - y)
			.abs()
			.total_cmp(&(b.1.center().y - y).abs())
	})?;
	let placement = if y < rect.top() + rect.height() / 3.0 {
		Placement::Before
	} else if y > rect.bottom() - rect.height() / 3.0 {
		Placement::After
	} else if matches!(source, Item::Folder(_)) {
		if y < rect.center().y {
			Placement::Before
		} else {
			Placement::After
		}
	} else {
		Placement::Inside
	};
	Some((item, rect, placement))
}

impl MessagingUi {
	pub(super) fn server_folders(
		&mut self,
		ui: &mut egui::Ui,
		state: &mut State,
		commands: &mut Vec<Command>,
	) {
		if self.folder_ui.generation != state.generation {
			// Folders the owner left open survive a restart and a session change.
			self.folder_ui = FolderUi {
				generation: state.generation,
				expanded: self.expanded_folders.iter().copied().collect(),
				..Default::default()
			};
		}
		if state.guild_folders.is_none()
			&& !state.folders_pending
			&& state.folders_error.is_none()
			&& (state.gateway_connected || state.demo)
			&& let Some(command) = state.load_guild_folders()
		{
			commands.push(command);
		}
		self.folder_ui.sync_rows(state);
		let colors = design::palette(ui);
		let enabled = state.guild_folders.is_some()
			&& !state.folders_pending
			&& (state.gateway_connected || state.demo);
		let mut change = None;
		let mut refresh = false;
		let mut drop_rows = Vec::new();
		let mut background: Option<(u64, egui::layers::ShapeIdx, egui::Rect, Color32)> = None;
		for index in 0..self.folder_ui.rows.len() {
			let (item, group) = self.folder_ui.rows[index];
			if group.map(|g| g.0) != background.as_ref().map(|b| b.0) {
				background = group.map(|(id, rgb)| {
					let tint = Color32::from_rgb((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8);
					(
						id,
						ui.painter().add(egui::Shape::Noop),
						egui::Rect::NOTHING,
						colors.raised.lerp_to_gamma(tint, 0.18),
					)
				});
			}
			let row = ui.push_id(
				match item {
					Item::Server(id) => (0, id.0),
					Item::Folder(id) => (1, id),
				},
				|ui| {
					if egui::DragAndDrop::payload::<Item>(ui.ctx())
						.is_some_and(|dragged| *dragged == item)
					{
						ui.set_opacity(0.0);
					}
					let response = match item {
						Item::Server(id) => {
							let Some(guild) = state.guilds.iter().find(|g| g.id == id) else {
								return;
							};
							let response = self.avatars.show_guild(
								ui,
								guild,
								self.guild == Some(id),
								state.demo,
							);
							let (unread, count) = self.rail_cache.guild_badge(id);
							rail_indicator(
								ui,
								response.rect,
								self.guild == Some(id),
								response.hovered() || response.has_focus(),
								unread,
							);
							if count > 0 {
								badge(
									ui,
									response.rect.right_bottom() - egui::vec2(8.0, 8.0),
									count,
									colors.base,
								);
							}
							if response.clicked() {
								self.guild = Some(id);
							}
							response
						}
						Item::Folder(id) => {
							let folder = state
								.guild_folders
								.as_ref()
								.unwrap()
								.folders
								.iter()
								.find(|f| f.id == Some(id))
								.unwrap();
							let rgb = folder.color.unwrap_or(design::DEFAULT_PRIMARY_RGB);
							let tint =
								Color32::from_rgb((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8);
							let (rect, response) = ui.allocate_exact_size(
								egui::Vec2::splat(48.0),
								Sense::click_and_drag(),
							);
							let open = self.folder_ui.expanded.contains(&id);
							if !open {
								ui.painter().rect_filled(rect, 12, colors.raised);
							}
							icons::paint(
								ui.painter(),
								if open { Icon::FolderOpen } else { Icon::Folder },
								rect.shrink(9.0),
								tint,
							);

							let unread = folder
								.guild_ids
								.iter()
								.any(|g| self.rail_cache.guild_badge(*g).0);
							let count = folder.guild_ids.iter().fold(0u32, |sum, g| {
								sum.saturating_add(self.rail_cache.guild_badge(*g).1)
							});
							if !open {
								rail_indicator(
									ui,
									rect,
									false,
									response.hovered() || response.has_focus(),
									unread,
								);
							}
							if count > 0 {
								badge(
									ui,
									rect.right_bottom() - egui::vec2(8.0, 8.0),
									count,
									colors.base,
								);
							}
							let name = folder.name.as_deref().unwrap_or("Server folder");
							response.widget_info(|| {
								egui::WidgetInfo::labeled(
									egui::WidgetType::Button,
									true,
									format!(
										"{name}, {} servers, {}",
										folder.guild_ids.len(),
										if open { "expanded" } else { "collapsed" }
									),
								)
							});
							if response.clicked() {
								self.folder_ui.toggle(id);
								// Bounded mirror of the open set, saved as a device preference.
								self.expanded_folders =
									self.folder_ui.expanded.iter().copied().take(256).collect();
							}
							response.on_hover_text_with(|| {
								format!("{name} · {} servers", folder.guild_ids.len())
							})
						}
					};
					response.context_menu(|ui| {
						if let Item::Server(id) = item {
							ui.set_width(232.0);
							let settings = self.server_menu.settings_item(ui, state, id);
							let leave = self.server_menu.leave_item(ui, state, id);
							if settings || leave {
								self.guild = Some(id);
							}
							ui.separator();
						}
						if ui
							.add_enabled(
								!state.folders_pending,
								egui::Button::new("Refresh folders from Discord"),
							)
							.clicked()
						{
							refresh = true;
							ui.close();
						}
						ui.add_enabled_ui(enabled, |ui| {
							if ui.button("Move up").clicked() {
								change = Some(Edit::Shift(item, false));
								ui.close();
							}
							if ui.button("Move down").clicked() {
								change = Some(Edit::Shift(item, true));
								ui.close();
							}
							match item {
								Item::Folder(id) => {
									if ui.button("Folder name and color…").clicked() {
										let f = state
											.guild_folders
											.as_ref()
											.unwrap()
											.folders
											.iter()
											.find(|f| f.id == Some(id))
											.unwrap();
										let color = f.color.unwrap_or(design::DEFAULT_PRIMARY_RGB);
										self.folder_ui.editor = Some((
											id,
											f.name.clone().unwrap_or_default(),
											[(color >> 16) as u8, (color >> 8) as u8, color as u8],
										));
										ui.close();
									}
									if ui.button("Ungroup servers").clicked() {
										change = Some(Edit::Dissolve(id));
										ui.close();
									}
								}
								Item::Server(id) => {
									if ui.button("Move outside folders").clicked() {
										change = Some(Edit::Outside(id));
										ui.close();
									}
									ui.menu_button("Group with server", |ui| {
										for guild in state.guilds.iter().filter(|g| g.id != id) {
											if ui.button(&guild.name).clicked() {
												change = Some(Edit::Drop(
													item,
													Item::Server(guild.id),
													Placement::Inside,
												));
												ui.close();
											}
										}
									});
								}
							}
						});
					});
					if enabled {
						response.dnd_set_drag_payload(item);
					}
				},
			);
			drop_rows.push((item, row.response.rect));
			if let Some((_, shape, rect, fill)) = &mut background {
				*rect = rect.union(row.response.rect);
				ui.painter().set(
					*shape,
					egui::Shape::rect_filled(rect.expand(4.0), 16, *fill),
				);
			}
		}
		if enabled
			&& let Some(source) = egui::DragAndDrop::payload::<Item>(ui.ctx())
			&& let Some(pointer) = ui.ctx().pointer_hover_pos()
			&& ui.clip_rect().contains(pointer)
		{
			if let Some((target, rect, placement)) = drop_target(&drop_rows, *source, pointer.y)
				&& target != *source
			{
				if placement == Placement::Inside {
					ui.painter().rect_stroke(
						rect.expand(2.0),
						12,
						(2.0, colors.accent),
						egui::StrokeKind::Outside,
					);
				} else {
					let mut rect = rect;
					if (matches!(*source, Item::Folder(_)) || matches!(target, Item::Folder(_)))
						&& let Some(settings) = &state.guild_folders
						&& let Some(index) = entry(settings, target)
					{
						for (item, sibling) in &drop_rows {
							if entry(settings, *item) == Some(index) {
								rect = rect.union(*sibling);
							}
						}
					}
					let y = if placement == Placement::Before {
						rect.top() - 6.0
					} else {
						rect.bottom() + 6.0
					};
					ui.painter().hline(
						rect.x_range(),
						y.max(ui.clip_rect().top() + 2.0)
							.min(ui.clip_rect().bottom() - 2.0),
						(3.0, colors.accent),
					);
				}
				if ui.input(|i| i.pointer.any_released()) {
					egui::DragAndDrop::take_payload::<Item>(ui.ctx());
					change = Some(Edit::Drop(*source, target, placement));
				}
			}
			if ui.input(|i| i.pointer.primary_down()) {
				let clip = ui.clip_rect();
				let direction = if pointer.y < clip.top() + 28.0 {
					1.0
				} else if pointer.y > clip.bottom() - 28.0 {
					-1.0
				} else {
					0.0
				};
				if direction != 0.0 {
					ui.scroll_with_delta(egui::vec2(0.0, direction * 8.0));
					ui.ctx()
						.request_repaint_after(std::time::Duration::from_millis(16));
				}
			}
		}
		// Keep the moving icon in the rail; its original slot stays available for layout.
		if let Some(item) = egui::DragAndDrop::payload::<Item>(ui.ctx())
			&& let Some(pointer) = ui.ctx().pointer_hover_pos()
		{
			let clip = ui.clip_rect();
			let top = clip.top();
			let bottom = (clip.bottom() - 48.0).max(top);
			let position = egui::pos2(ui.max_rect().left(), (pointer.y - 24.0).clamp(top, bottom));
			ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
			egui::Area::new(egui::Id::unique("server-drag-preview"))
				.order(egui::Order::Tooltip)
				.fixed_pos(position)
				.interactable(false)
				.show(ui.ctx(), |ui| {
					ui.set_clip_rect(clip);
					match *item {
						Item::Server(id) => {
							if let Some(guild) = state.guilds.iter().find(|g| g.id == id) {
								self.avatars.show_guild(ui, guild, false, state.demo);
							}
						}
						Item::Folder(id) => {
							if let Some(folder) = state
								.guild_folders
								.as_ref()
								.and_then(|s| s.folders.iter().find(|f| f.id == Some(id)))
							{
								let rgb = folder.color.unwrap_or(design::DEFAULT_PRIMARY_RGB);
								let (rect, _) =
									ui.allocate_exact_size(egui::Vec2::splat(48.0), Sense::hover());
								ui.painter().rect_filled(rect, 12, colors.raised);
								icons::paint(
									ui.painter(),
									Icon::Folder,
									rect.shrink(9.0),
									Color32::from_rgb(
										(rgb >> 16) as u8,
										(rgb >> 8) as u8,
										rgb as u8,
									),
								);
							}
						}
					}
				});
		}
		if state.folders_pending {
			ui.label(egui::RichText::new("Sync…").small())
				.on_hover_text("Syncing server folders with Discord");
		}
		if let Some(error) = state.folders_error
			&& ui.small_button("Retry").on_hover_text(error).clicked()
			&& let Some(command) = state.load_guild_folders()
		{
			commands.push(command);
		}
		let mut close = false;
		if let Some((id, name, color)) = &mut self.folder_ui.editor {
			let response = crate::dialog::Dialog::new("folder-settings", "Folder Settings")
				.subtitle("Name this folder and pick the colour shown on the server rail.")
				.width(400.0)
				.show(ui.ctx(), |d| {
					d.content(|ui| {
						let label = crate::dialog::label(ui, "Folder name");
						crate::dialog::input(
							ui,
							egui::TextEdit::singleline(name)
								.hint_text("Folder name")
								.char_limit(100),
						)
						.labelled_by(label.id);
						ui.add_space(14.0);
						crate::dialog::label(ui, "Colour");
						design::color_edit(ui, color);
					});
					d.footer(|ui| {
						ui.add_enabled_ui(enabled, |ui| {
							if crate::dialog::action(ui, "Save", crate::dialog::Action::Primary)
								.clicked()
							{
								change = Some(Edit::Customize(
									*id,
									name.clone(),
									((color[0] as u32) << 16)
										| ((color[1] as u32) << 8) | color[2] as u32,
								));
								close = true;
							}
						});
						close |=
							crate::dialog::action(ui, "Cancel", crate::dialog::Action::Neutral)
								.clicked();
					});
				});
			close |= response.close;
		}
		if close {
			self.folder_ui.editor = None;
		}
		if refresh && let Some(command) = state.load_guild_folders() {
			commands.push(command);
		}
		if let Some(change) = change
			&& let Some(mut settings) = state.guild_folders.clone()
		{
			// Include newly joined servers without deleting unknown remote memberships.
			for guild in &state.guilds {
				if !settings
					.folders
					.iter()
					.any(|f| f.guild_ids.contains(&guild.id))
				{
					settings.folders.push(standalone(guild.id));
				}
			}
			edit(&mut settings, change);
			if let Some(command) = state.save_guild_folders(settings) {
				commands.push(command);
			}
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn folder_rows_cache_tracks_expansion_order_color_and_acknowledged_writes() {
		let settings = Settings {
			folders: vec![
				Folder {
					id: Some(7),
					guild_ids: vec![Id(1), Id(2)],
					name: Some("Synthetic folder".into()),
					color: Some(0x123456),
				},
				standalone(Id(3)),
			],
			..Default::default()
		};
		let mut state = State {
			demo: true,
			guilds: (1..=3)
				.map(|id| model::Guild {
					id: Id(id),
					name: "Synthetic server".into(),
					icon: None,
					emojis: None,
				})
				.collect(),
			guild_folders: Some(settings),
			..State::default()
		};
		let mut folders = FolderUi::default();
		assert!(folders.sync_rows(&state));
		assert_eq!(
			&*folders.rows,
			&[(Item::Folder(7), None), (Item::Server(Id(3)), None)]
		);
		for _ in 0..10 {
			assert!(!folders.sync_rows(&state));
		}
		folders.toggle(7);
		assert!(folders.sync_rows(&state));
		assert_eq!(
			&*folders.rows,
			&[
				(Item::Folder(7), Some((7, 0x123456))),
				(Item::Server(Id(1)), Some((7, 0x123456))),
				(Item::Server(Id(2)), Some((7, 0x123456))),
				(Item::Server(Id(3)), None),
			]
		);
		let mut changed = state.guild_folders.clone().unwrap();
		edit(&mut changed, Edit::Shift(Item::Server(Id(2)), false));
		edit(
			&mut changed,
			Edit::Customize(7, "New name".into(), 0xabcdef),
		);
		state.save_guild_folders(changed);
		assert!(folders.sync_rows(&state));
		assert_eq!(folders.rows[1], (Item::Server(Id(2)), Some((7, 0xabcdef))));
		assert_eq!(folders.rows[2], (Item::Server(Id(1)), Some((7, 0xabcdef))));
		let original = folders.rows.clone();
		state.demo = false;
		state.auth = client_core::auth::AuthState::Authenticated;
		state.gateway_connected = true;
		let mut changed = state.guild_folders.clone().unwrap();
		edit(&mut changed, Edit::Dissolve(7));
		let command = state.save_guild_folders(changed.clone()).unwrap();
		assert!(!folders.sync_rows(&state));
		state.command_rejected(command);
		assert!(!folders.sync_rows(&state));
		assert!(state.folders_error.is_some());
		assert_eq!(folders.rows, original);
		assert!(state.save_guild_folders(changed.clone()).is_some());
		state.apply(client_core::Envelope {
			generation: state.generation,
			event: client_core::Event::GuildFolders(Ok(changed)),
		});
		assert!(folders.sync_rows(&state));
		assert_eq!(
			&*folders.rows,
			&[
				(Item::Server(Id(2)), None),
				(Item::Server(Id(1)), None),
				(Item::Server(Id(3)), None),
			]
		);
		assert!(folders.expanded.is_empty());
		let original = folders.rows.clone();
		assert!(state.load_guild_folders().is_some());
		state.apply(client_core::Envelope {
			generation: state.generation,
			event: client_core::Event::GuildFolders(Err(client_core::auth::Failure::Network)),
		});
		assert!(folders.sync_rows(&state));
		assert!(state.folders_error.is_some());
		assert_eq!(folders.rows, original);
		state.logout();
		assert!(folders.sync_rows(&state));
		assert!(folders.rows.is_empty());
	}

	#[test]
	fn wide_drop_zones_and_order_preserve_folder_membership() {
		let first = Item::Server(Id(1));
		let second = Item::Server(Id(2));
		let rows = [
			(
				first,
				egui::Rect::from_min_size(egui::pos2(12.0, 100.0), egui::Vec2::splat(48.0)),
			),
			(
				second,
				egui::Rect::from_min_size(egui::pos2(12.0, 160.0), egui::Vec2::splat(48.0)),
			),
		];
		for (y, expected) in [
			(40.0, Placement::Before),
			(114.0, Placement::Before),
			(124.0, Placement::Inside),
			(150.0, Placement::After),
			(250.0, Placement::After),
		] {
			assert_eq!(
				drop_target(&rows, Item::Server(Id(3)), y).unwrap().2,
				expected
			);
		}
		let mut settings = Settings {
			folders: vec![standalone(Id(1)), standalone(Id(2)), standalone(Id(3))],
			..Default::default()
		};
		edit(
			&mut settings,
			Edit::Drop(Item::Server(Id(3)), first, Placement::Before),
		);
		assert_eq!(
			settings
				.folders
				.iter()
				.flat_map(|f| &f.guild_ids)
				.copied()
				.collect::<Vec<_>>(),
			vec![Id(3), Id(1), Id(2)]
		);
		edit(
			&mut settings,
			Edit::Drop(Item::Server(Id(3)), second, Placement::After),
		);
		assert_eq!(
			settings
				.folders
				.iter()
				.flat_map(|f| &f.guild_ids)
				.copied()
				.collect::<Vec<_>>(),
			vec![Id(1), Id(2), Id(3)]
		);
		edit(&mut settings, Edit::Drop(first, second, Placement::Inside));
		let folder = settings.folders[0].id;
		edit(&mut settings, Edit::Drop(first, second, Placement::Before));
		assert_eq!(settings.folders[0].id, folder);
		assert_eq!(settings.folders[0].guild_ids, vec![Id(1), Id(2)]);
		assert!(settings.valid());
	}
}
