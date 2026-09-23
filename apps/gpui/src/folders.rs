//! Server folders on the rail from the account's `guild_folders` settings, as in egui's
//! `guild_folders.rs`: collapsed tiles show a 2×2 mosaic, expanded folders sit on a tinted plate.
//! Expansion is session-only; reordering and folder editing stay in the main app.
use crate::sidebar::{Tile, initials};
use crate::theme::{Icon, color, icon, palette};
use crate::{Serein, tooltip};
use client_core::State;
use gpui::{prelude::*, *};
use model::Id;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

const MOSAIC: usize = 4;
const MOSAIC_ICON: f32 = 18.;
const MOSAIC_GAP: f32 = 2.;
/// Rail entries rendered at most, like the egui row cap.
const MAX_ENTRIES: usize = client_core::MAX_NAV + model::guild_folders::MAX_FOLDERS;

#[derive(Clone, Debug, PartialEq)]
pub enum Entry {
	Server(Id),
	Folder {
		id: u64,
		name: String,
		rgb: u32,
		open: bool,
		guilds: Vec<Id>,
	},
}

/// Rail order: servers missing from folder settings first (freshly joined, as Discord does), then
/// the settings order. Folder members Serein does not know are skipped; empty folders vanish.
pub fn entries(state: &State, expanded: &BTreeSet<u64>) -> Vec<Entry> {
	let mut seen = BTreeSet::new();
	if let Some(settings) = &state.guild_folders {
		for folder in &settings.folders {
			seen.extend(folder.guild_ids.iter().copied());
		}
	}
	let mut rows = state
		.guilds
		.iter()
		.filter(|guild| !seen.contains(&guild.id))
		.map(|guild| Entry::Server(guild.id))
		.collect::<Vec<_>>();
	let mut placed = BTreeSet::new();
	for folder in state.guild_folders.iter().flat_map(|s| &s.folders) {
		let guilds = folder
			.guild_ids
			.iter()
			.copied()
			.filter(|id| state.guild(*id).is_some() && placed.insert(*id))
			.collect::<Vec<_>>();
		match folder.id {
			Some(id) if !guilds.is_empty() => rows.push(Entry::Folder {
				id,
				name: folder
					.name
					.clone()
					.filter(|name| !name.trim().is_empty())
					.unwrap_or_else(|| "Server folder".into()),
				rgb: folder.color.unwrap_or(ui::design::DEFAULT_PRIMARY_RGB),
				open: expanded.contains(&id),
				guilds,
			}),
			Some(_) => {}
			None => rows.extend(guilds.into_iter().map(Entry::Server)),
		}
	}
	rows.truncate(MAX_ENTRIES);
	rows
}

fn rgb_color(rgb: u32) -> egui::Color32 {
	egui::Color32::from_rgb((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8)
}

/// Initials and loaded icon of the first four known servers, for the collapsed folder tile.
fn mosaic(state: &State, guilds: &[Id]) -> Vec<(String, Option<Arc<RenderImage>>)> {
	guilds
		.iter()
		.filter_map(|id| state.guild(*id))
		.take(MOSAIC)
		.map(|guild| {
			(
				initials(&guild.name),
				guild.icon_key().and_then(|key| crate::images::get(&key)),
			)
		})
		.collect()
}

fn mosaic_face(label: &str, image: Option<&Arc<RenderImage>>) -> AnyElement {
	let p = palette();
	let face = div()
		.size(px(MOSAIC_ICON))
		.flex_none()
		.rounded_full()
		.overflow_hidden();
	match image {
		Some(image) => face
			.child(
				img(image.clone())
					.size_full()
					.rounded_full()
					.object_fit(ObjectFit::Cover),
			)
			.into_any_element(),
		None => face
			.bg(color(p.raised))
			.flex()
			.items_center()
			.justify_center()
			.text_size(px(7.))
			.font_weight(FontWeight::SEMIBOLD)
			.text_color(color(p.text_strong))
			.child(label.to_owned())
			.into_any_element(),
	}
}

impl Serein {
	fn toggle_folder(&mut self, id: u64, cx: &mut Context<Self>) {
		if !self.navigation.expanded.remove(&id) {
			// Session-only and bounded like the egui preference list.
			if self.navigation.expanded.len() < 256 {
				self.navigation.expanded.insert(id);
			}
		}
		cx.notify();
	}

	/// One folder: a mosaic tile while collapsed, or its open header and servers on a tinted plate.
	pub(crate) fn folder_entry(
		&self,
		entry: &Entry,
		badges: &BTreeMap<Id, (bool, u32)>,
		cx: &mut Context<Self>,
	) -> AnyElement {
		let p = palette();
		let Entry::Folder {
			id,
			name,
			rgb,
			open,
			guilds,
		} = entry
		else {
			return div().into_any_element();
		};
		let (id, open, tint) = (*id, *open, rgb_color(*rgb));
		let label = format!(
			"{name} · {} server{}",
			guilds.len(),
			if guilds.len() == 1 { "" } else { "s" }
		);
		if open {
			let header = div()
				.id(("folder", id))
				.focusable()
				.tab_stop(true)
				.size(px(46.))
				.flex_none()
				.rounded(px(13.))
				.flex()
				.items_center()
				.justify_center()
				.cursor_pointer()
				.hover(|d| d.bg(color(p.hover)))
				.focus(|d| d.border_2().border_color(color(p.text_strong)))
				.tooltip(tooltip(label))
				.on_click(cx.listener(move |this, _, _, cx| this.toggle_folder(id, cx)))
				.child(icon(Icon::FolderOpen, px(29.), color(tint)));
			let mut column = div()
				.relative()
				.w_full()
				.py(px(4.))
				.flex()
				.flex_col()
				.items_center()
				.gap(px(12.))
				.child(
					div()
						.absolute()
						.top_0()
						.bottom_0()
						.left(px(7.))
						.right(px(7.))
						.rounded(px(16.))
						.bg(color(p.raised.lerp_to_gamma(tint, 0.18))),
				)
				.child(header);
			for guild in guilds {
				column = column.child(self.guild_tile(*guild, badges, cx));
			}
			return column.into_any_element();
		}
		let unread = guilds.iter().any(|g| badges.get(g).is_some_and(|b| b.0));
		let count = guilds.iter().fold(0u32, |sum, g| {
			sum.saturating_add(badges.get(g).map_or(0, |b| b.1))
		});
		let faces = mosaic(&self.state, guilds);
		self.rail_tile(
			Tile {
				id: ("folder", id).into(),
				name: label.into(),
				selected: false,
				unread,
				badge: count,
				size: 46.,
				accent_hover: false,
				menu: None,
			},
			move |this, cx| this.toggle_folder(id, cx),
			move |highlight| {
				let lighter = tint.lerp_to_gamma(egui::Color32::WHITE, 0.1);
				div()
					.size_full()
					.rounded(px(13.))
					.bg(color(if highlight { lighter } else { tint }))
					.hover(move |d| d.bg(color(lighter)))
					.p(px(4.))
					.flex()
					.flex_wrap()
					.content_start()
					.gap(px(MOSAIC_GAP))
					.children(
						faces
							.iter()
							.map(|(label, image)| mosaic_face(label, image.as_ref())),
					)
			},
			cx,
		)
	}
}

#[cfg(test)]
mod tests {
	use super::{Entry, entries};
	use model::Id;
	use model::guild_folders::{Folder, Settings};
	use std::collections::BTreeSet;

	#[test]
	fn unfoldered_servers_lead_and_folders_follow_settings_order() {
		let mut state = test_support::demo_state();
		test_support::seed_demo_folder_mosaic(&mut state);
		let collapsed = entries(&state, &BTreeSet::new());
		assert_eq!(collapsed.len(), 2);
		assert!(matches!(
			&collapsed[0],
			Entry::Folder { id: 1, open: false, guilds, rgb: 0x5865f2, .. } if guilds.len() == 4
		));
		assert_eq!(collapsed[1], Entry::Server(Id(10)));
		let open = entries(&state, &BTreeSet::from([1]));
		assert!(matches!(&open[0], Entry::Folder { open: true, .. }));
		// A server missing from the settings is new and goes first.
		state.guild_folders = Some(Settings {
			folders: vec![Folder {
				id: Some(9),
				guild_ids: vec![Id(11), Id(404)],
				name: Some("  ".into()),
				color: None,
			}],
			version: 0,
		});
		let rows = entries(&state, &BTreeSet::new());
		assert_eq!(
			rows.iter()
				.filter(|row| matches!(row, Entry::Server(_)))
				.count(),
			4
		);
		assert_eq!(rows[0], Entry::Server(Id(10)));
		let Some(Entry::Folder {
			name, rgb, guilds, ..
		}) = rows.last()
		else {
			panic!("folder row");
		};
		assert_eq!(name, "Server folder");
		assert_eq!(*rgb, ui::design::DEFAULT_PRIMARY_RGB);
		assert_eq!(guilds, &[Id(11)], "unknown servers are skipped");
	}

	#[test]
	fn no_settings_lists_every_server_once() {
		let mut state = test_support::demo_state();
		test_support::seed_demo_folder_mosaic(&mut state);
		state.guild_folders = None;
		let rows = entries(&state, &BTreeSet::new());
		assert_eq!(rows.len(), state.guilds.len());
		assert!(rows.iter().all(|row| matches!(row, Entry::Server(_))));
	}
}
