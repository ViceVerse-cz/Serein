//! Sticker browsing shares the composer popout and the bounded image working set.
use crate::{avatars::Avatars, design};
use client_core::State;
use model::{Id, Sticker};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Group {
	Recent,
	Guild(Id),
	Pack(Id),
}

#[derive(Default)]
pub(crate) struct Browser {
	pub query: String,
	pub target: Option<Group>,
}

struct Section<'a> {
	id: Group,
	name: &'a str,
	stickers: &'a [Sticker],
}

fn sections(state: &State) -> Vec<Section<'_>> {
	let mut sections = Vec::new();
	if !state.stickers.recent.is_empty() {
		sections.push(Section {
			id: Group::Recent,
			name: "Recently Used",
			stickers: &state.stickers.recent,
		});
	}
	sections.extend(state.guilds.iter().filter_map(|guild| {
		let stickers = guild
			.stickers
			.as_deref()
			.filter(|items| !items.is_empty())?;
		Some(Section {
			id: Group::Guild(guild.id),
			name: &guild.name,
			stickers,
		})
	}));
	sections.extend(state.stickers.packs.iter().map(|pack| Section {
		id: Group::Pack(pack.id),
		name: &pack.name,
		stickers: &pack.stickers,
	}));
	sections
}

fn matches(sticker: &Sticker, source: &str, query: &str) -> bool {
	query.is_empty()
		|| [&sticker.name, &sticker.tags, source]
			.iter()
			.any(|text| text.to_lowercase().contains(query))
}

impl Browser {
	pub fn focus(&mut self, sticker: &Sticker) {
		self.query.clear();
		self.target = sticker
			.guild_id
			.map(Group::Guild)
			.or(sticker.pack_id.map(Group::Pack));
	}

	pub fn show(
		&mut self,
		ui: &mut egui::Ui,
		state: &State,
		avatars: &mut Avatars,
		hovered: &mut Option<(Sticker, String)>,
		as_image: bool,
	) -> Option<Sticker> {
		let colors = design::palette(ui);
		let sections = sections(state);
		let query = self.query.trim().to_lowercase();
		let mut chosen = None;
		ui.horizontal_top(|ui| {
			ui.vertical(|ui| {
				ui.set_width(40.0);
				egui::ScrollArea::vertical()
					.id_salt("sticker-rail")
					.show(ui, |ui| {
						for section in &sections {
							let response = ui
								.push_id(section.id, |ui| match section.id {
									Group::Guild(id) => avatars.show_guild_sized(
										ui,
										state.guild(id).expect("section belongs to guild"),
										false,
										state.demo,
										32.0,
									),
									Group::Pack(_) if !section.stickers.is_empty() => avatars
										.sticker_image(
											ui,
											&section.stickers[0],
											egui::Vec2::splat(32.0),
											state.demo,
										)
										.on_hover_text(section.name),
									_ => ui
										.add_sized([32.0, 32.0], egui::Button::new("◷"))
										.on_hover_text(section.name),
								})
								.inner;
							if response.clicked() {
								self.target = Some(section.id);
								self.query.clear();
							}
						}
					});
			});
			ui.separator();
			ui.vertical(|ui| {
				ui.set_width(ui.available_width());
				let columns = (ui.available_width() / 104.0).floor().max(1.0) as usize;
				let size = ((ui.available_width()
					- (columns - 1) as f32 * ui.spacing().item_spacing.x)
					/ columns as f32)
					.min(120.0);
				let mut results = 0;
				egui::ScrollArea::vertical()
					.id_salt("sticker-groups")
					.auto_shrink([false, false])
					.show(ui, |ui| {
						for section in &sections {
							// Bound search result allocation independently of the account catalog.
							let matching: Vec<_> = section
								.stickers
								.iter()
								.filter(|s| matches(s, section.name, &query))
								.take(if query.is_empty() {
									500
								} else {
									500usize.saturating_sub(results)
								})
								.collect();
							if !query.is_empty() && matching.is_empty() {
								continue;
							}
							results += matching.len();
							let target = self.target == Some(section.id);
							let mut collapse =
								egui::collapsing_header::CollapsingState::load_with_default_open(
									ui.ctx(),
									ui.make_persistent_id(section.id),
									true,
								);
							if target {
								collapse.set_open(true);
							}
							let mut toggle = false;
							let mut header = collapse.show_header(ui, |ui| {
								if let Group::Guild(id) = section.id
									&& let Some(guild) = state.guild(id)
								{
									let (rect, response) = ui.allocate_exact_size(
										egui::Vec2::splat(20.0),
										egui::Sense::click(),
									);
									avatars.paint_guild(ui, guild, rect, state.demo, 6);
									toggle |= response.clicked();
								}
								toggle |= ui
									.add(
										egui::Label::new(design::semibold(ui, section.name, 14.0))
											.sense(egui::Sense::click()),
									)
									.clicked();
							});
							if toggle {
								header.toggle();
							}
							let (_, header, _) = header.body(|ui| {
								if matching.is_empty() {
									ui.label(
										egui::RichText::new("This server has no stickers yet.")
											.color(colors.muted),
									);
								}
								for row in matching.chunks(columns) {
									if !ui.is_rect_visible(egui::Rect::from_min_size(
										ui.cursor().min,
										egui::vec2(ui.available_width(), size),
									)) {
										ui.allocate_space(egui::vec2(ui.available_width(), size));
										continue;
									}
									ui.horizontal(|ui| {
										for sticker in row {
											ui.push_id(sticker.id, |ui| {
												let enabled = if as_image {
													sticker.valid()
														&& state.selected.is_some_and(|channel| {
															state.can_send(channel)
																&& state.can_attach(channel)
														})
												} else {
													state.can_send_sticker(sticker)
												};
												let response = ui
													.add_enabled_ui(enabled, |ui| {
														avatars.sticker_image(
															ui,
															sticker,
															egui::Vec2::splat(size),
															state.demo,
														)
													})
													.inner;
												if response.contains_pointer()
													|| response.has_focus()
												{
													*hovered = Some((
														(*sticker).clone(),
														section.name.to_owned(),
													));
												}
												if response.clicked() {
													chosen = Some((*sticker).clone());
												}
												if !enabled {
													response.on_disabled_hover_text(
														if !as_image
															&& state.sticker_requires_nitro(sticker)
														{
															"Nitro is required to use this sticker outside its server."
														} else {
															"This sticker is unavailable with the current connection or permissions."
														},
													);
												}
											});
										}
									});
								}
							});
							if target {
								header.response.scroll_to_me(Some(egui::Align::Min));
								self.target = None;
							}
							if !query.is_empty() && results >= 500 {
								ui.small(
									"Showing the first 500 stickers. Search to narrow the results.",
								);
								break;
							}
						}
						if results == 0 && !query.is_empty() {
							ui.label("No stickers found.");
						}
						if state.stickers.loading {
							ui.label("Loading sticker packs…");
						}
					});
			});
		});
		chosen
	}
}

/// Message artwork and an anchored details card. The owner dispatches explicit reads.
pub(crate) fn message(
	ui: &mut egui::Ui,
	sticker: &Sticker,
	state: &State,
	avatars: &mut Avatars,
	request: &mut Option<Id>,
	browse: &mut Option<Sticker>,
) -> egui::Response {
	let edge = ui.available_width().min(160.0);
	let response = avatars.sticker_image(ui, sticker, egui::Vec2::splat(edge), state.demo);
	if response.clicked() {
		*request = Some(sticker.id);
	}
	let colors = design::palette(ui);
	let width = 340.0_f32.min((ui.ctx().content_rect().width() - 40.0).max(160.0));
	egui::Popup::from_toggle_button_response(&response)
		.id(response.id.with("sticker-details"))
		.close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
		.gap(6.0)
		.width(width)
		.frame(
			egui::Frame::popup(ui.style())
				.fill(colors.raised)
				.inner_margin(16)
				.corner_radius(10),
		)
		.show(|ui| {
			ui.set_width(width - 32.0);
			ui.spacing_mut().item_spacing.y = 8.0;
			let detail = state
				.stickers
				.detail
				.as_ref()
				.filter(|detail| detail.id == sticker.id)
				.unwrap_or(sticker);
			let sections = sections(state);
			let section = sections
				.iter()
				.find(|section| match section.id {
					Group::Guild(id) => detail.guild_id == Some(id),
					Group::Pack(id) => detail.pack_id == Some(id),
					Group::Recent => false,
				})
				.or_else(|| {
					sections.iter().find(|section| {
						section.id != Group::Recent
							&& section.stickers.iter().any(|s| s.id == sticker.id)
					})
				});
			ui.add(
				egui::Label::new(
					design::semibold(ui, &detail.name, 16.0).color(colors.text_strong),
				)
				.wrap(),
			);
			if let Some(section) = section {
				ui.label(format!("This is a {} sticker.", section.name));
			} else if state.stickers.detail_loading == Some(sticker.id) {
				ui.label("Loading sticker details…");
			} else {
				ui.label(
					state
						.stickers
						.detail_error
						.unwrap_or("Sticker details unavailable."),
				);
				if ui.button("Retry sticker details").clicked() {
					*request = Some(sticker.id);
				}
			}
			if !detail.description.is_empty() {
				egui::ScrollArea::vertical()
					.max_height(120.0)
					.show(ui, |ui| {
						ui.label(
							egui::RichText::new(&detail.description)
								.size(14.0)
								.color(colors.muted),
						);
					});
			}
			if let Some(section) = section {
				let edge =
					((ui.available_width() - 2.0 * ui.spacing().item_spacing.x) / 3.0).min(88.0);
				ui.horizontal(|ui| {
					for other in section.stickers.iter().take(3) {
						ui.push_id(other.id, |ui| {
							avatars
								.sticker_image(ui, other, egui::Vec2::splat(edge), state.demo)
								.on_hover_text(&other.name);
						});
					}
				});
			}
			ui.separator();
			if design::secondary_button(ui, "View More Stickers").clicked() {
				*browse = Some(detail.clone());
				ui.close();
			}
		});
	response
}

#[cfg(test)]
mod tests {
	use super::*;
	fn sticker(id: u64, pack: u64) -> Sticker {
		Sticker {
			id: Id(id),
			name: "Wave".into(),
			tags: "hello".into(),
			description: String::new(),
			format_type: 1,
			guild_id: None,
			pack_id: Some(Id(pack)),
			available: true,
		}
	}
	fn label_rect(shape: &egui::Shape, label: &str) -> Option<egui::Rect> {
		match shape {
			egui::Shape::Text(text) if text.galley.job.text == label => {
				Some(egui::Rect::from_min_size(text.pos, text.galley.size()))
			}
			egui::Shape::Vec(shapes) => shapes.iter().find_map(|shape| label_rect(shape, label)),
			_ => None,
		}
	}
	#[test]
	fn sticker_details_open_and_view_more_keeps_pack_provenance() {
		for dark in [false, true] {
			let ctx = egui::Context::default();
			ctx.set_visuals(if dark {
				egui::Visuals::dark()
			} else {
				egui::Visuals::light()
			});
			let detail = sticker(7, 8);
			let mut item = detail.clone();
			item.pack_id = None; // Message items do not carry full catalog metadata.
			let mut state = State {
				demo: true,
				..State::default()
			};
			state.stickers.detail = Some(detail.clone());
			state.stickers.packs.push(model::StickerPack {
				id: Id(8),
				name: "Synthetic friends".into(),
				stickers: (7..10).map(|id| sticker(id, 8)).collect(),
			});
			let mut avatars = Avatars::default();
			let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(280.0, 600.0));
			let mut frame = |events| {
				let (mut request, mut browse, mut rect) = (None, None, egui::Rect::NOTHING);
				let output = ctx.run_ui(
					egui::RawInput {
						screen_rect: Some(screen),
						events,
						..Default::default()
					},
					|ui| {
						rect = message(ui, &item, &state, &mut avatars, &mut request, &mut browse)
							.rect;
					},
				);
				(output, request, browse, rect)
			};
			let click = |pos, pressed| {
				vec![
					egui::Event::PointerMoved(pos),
					egui::Event::PointerButton {
						pos,
						button: egui::PointerButton::Primary,
						pressed,
						modifiers: egui::Modifiers::NONE,
					},
				]
			};
			let mut artwork = egui::Rect::NOTHING;
			for _ in 0..3 {
				let (output, _, _, rect) = frame(vec![]);
				artwork = rect;
				output.drop_without_applying_deltas();
			}
			frame(click(artwork.center(), true))
				.0
				.drop_without_applying_deltas();
			let (output, request, _, _) = frame(click(artwork.center(), false));
			assert_eq!(request, Some(item.id));
			output.drop_without_applying_deltas();
			for _ in 0..2 {
				frame(vec![]).0.drop_without_applying_deltas();
			}
			let (output, _, _, _) = frame(vec![]);
			let find = |label| {
				output
					.shapes
					.iter()
					.find_map(|shape| label_rect(&shape.shape, label))
					.expect(label)
			};
			assert!(screen.contains_rect(find("This is a Synthetic friends sticker.")));
			let more = find("View More Stickers");
			assert!(screen.contains_rect(more));
			output.drop_without_applying_deltas();
			frame(click(more.center(), true))
				.0
				.drop_without_applying_deltas();
			let (output, _, browse, _) = frame(click(more.center(), false));
			assert_eq!(browse, Some(detail));
			output.drop_without_applying_deltas();
			assert!(avatars.take_requests().is_empty());
		}
	}
	#[test]
	fn rail_target_after_five_hundred_stickers_is_reached() {
		let ctx = egui::Context::default();
		let mut state = State {
			demo: true,
			..State::default()
		};
		state.stickers.packs = vec![
			model::StickerPack {
				id: Id(1),
				name: "First".into(),
				stickers: (1..=500).map(|id| sticker(id, 1)).collect(),
			},
			model::StickerPack {
				id: Id(2),
				name: "Last".into(),
				stickers: vec![sticker(501, 2)],
			},
		];
		let mut browser = Browser {
			query: "old query".into(),
			..Default::default()
		};
		browser.focus(&state.stickers.packs[1].stickers[0]);
		let mut avatars = Avatars::default();
		ctx.run_ui(
			egui::RawInput {
				screen_rect: Some(egui::Rect::from_min_size(
					egui::Pos2::ZERO,
					egui::vec2(400.0, 400.0),
				)),
				..Default::default()
			},
			|ui| {
				browser.show(ui, &state, &mut avatars, &mut None, false);
			},
		)
		.drop_without_applying_deltas();
		assert!(browser.query.is_empty());
		assert_eq!(browser.target, None);
		assert!(avatars.take_requests().is_empty());
	}
	#[test]
	fn search_matches_names_tags_and_source() {
		let sticker = Sticker {
			id: Id(1),
			name: "Wave".into(),
			tags: "hello,greeting".into(),
			description: String::new(),
			format_type: 1,
			guild_id: Some(Id(2)),
			pack_id: None,
			available: true,
		};
		assert!(matches(&sticker, "Cozy Club", "wave"));
		assert!(matches(&sticker, "Cozy Club", "hello"));
		assert!(matches(&sticker, "Cozy Club", "cozy"));
		assert!(!matches(&sticker, "Cozy Club", "sleep"));
	}
}
