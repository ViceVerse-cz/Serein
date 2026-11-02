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
		image_fallback: bool,
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
										egui::RichText::new(crate::i18n::translate(
											"stickers-show-this-server-has-no-stickers-yet",
										))
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
												let enabled = state.can_send_sticker(sticker)
													|| (image_fallback
														&& sticker.valid() && sticker.available
														&& state.selected.is_some_and(|channel| {
															state.can_send(channel)
														}));
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
														crate::i18n::translate_if_key(
															if !image_fallback
																&& state
																	.sticker_requires_nitro(sticker)
															{
																"stickers-show-nitro-is-required-to-use-this-sticker-outside-its-server"
															} else {
																"stickers-show-this-sticker-is-unavailable-with-the-current-connection-or-permissions"
															},
														),
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
								ui.small(crate::i18n::translate(
									"stickers-show-showing-the-first-500-stickers-search-to-narrow-the-results",
								));
								break;
							}
						}
						if results == 0 && !query.is_empty() {
							ui.label(crate::i18n::translate("stickers-show-no-stickers-found"));
						}
						if state.stickers.loading {
							ui.label(crate::i18n::translate("stickers-show-loading-sticker-packs"));
						}
					});
			});
		});
		chosen
	}
}

/// What a received sticker's details card says about its source.
#[derive(Debug, PartialEq)]
enum Provenance<'a> {
	/// A joined server or a loaded standard pack.
	Named(&'a str),
	Loading,
	/// Details name a server this session cannot see, such as an invite-only one.
	OtherServer,
	/// Details could not be loaded, with the reason when known; the card offers a retry.
	Unavailable(Option<&'static str>),
}

fn provenance<'a>(
	source: Option<&'a str>,
	detail: &Sticker,
	loading: bool,
	error: Option<&'static str>,
) -> Provenance<'a> {
	match source {
		Some(name) => Provenance::Named(name),
		None if loading => Provenance::Loading,
		None if detail.guild_id.is_some() => Provenance::OtherServer,
		None => Provenance::Unavailable(error),
	}
}

/// A details line followed by the compact retry icon.
fn retry_row(ui: &mut egui::Ui, text: String, id: Id, request: &mut Option<Id>) {
	ui.horizontal_top(|ui| {
		ui.spacing_mut().item_spacing.x = 4.0;
		let retry = 24.0;
		let width = (ui.available_width() - retry - 4.0).max(40.0);
		ui.allocate_ui(egui::vec2(width, retry), |ui| {
			ui.add(egui::Label::new(text).wrap());
		});
		if crate::icons::button(
			ui,
			crate::icons::Icon::Reload,
			retry,
			"stickers-message-retry-sticker-details",
		)
		.clicked()
		{
			*request = Some(id);
		}
	});
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
	let mut response = avatars
		.sticker_image(ui, sticker, egui::Vec2::splat(edge), state.demo)
		.on_hover_cursor(egui::CursorIcon::PointingHand);
	if response.hovered() {
		let art = avatars.share_image(
			ui.ctx(),
			model::ImageShare::Sticker {
				id: sticker.id,
				format_type: sticker.format_type,
			},
			crate::emoji_details::HOVER_ART,
			state.demo,
		);
		response = crate::emoji_details::hover_card(response, art, &sticker.name, true);
	}
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
			// A joined server names the source even when its catalog no longer lists the sticker.
			let source = section.map(|section| section.name).or_else(|| {
				detail
					.guild_id
					.and_then(|id| state.guild(id))
					.map(|guild| guild.name.as_str())
			});
			let loading = state.stickers.detail_loading == Some(sticker.id);
			let provenance = provenance(source, detail, loading, state.stickers.detail_error);
			// A failed reload keeps the known source but still offers the retry.
			let failed = !loading && state.stickers.detail_error.is_some();
			ui.horizontal_top(|ui| {
				ui.spacing_mut().item_spacing.x = 12.0;
				avatars.sticker_image(ui, sticker, egui::Vec2::splat(64.0), state.demo);
				ui.vertical(|ui| {
					ui.spacing_mut().item_spacing.y = 4.0;
					ui.set_width(ui.available_width());
					ui.add(
						egui::Label::new(
							design::semibold(ui, &detail.name, 16.0).color(colors.text_strong),
						)
						.wrap(),
					);
					let line = |text: String| egui::Label::new(text).wrap();
					match provenance {
						Provenance::Named(name) => {
							ui.add(line(format!(
								"{} {} {}.",
								crate::i18n::translate("stickers-message-this-is-a"),
								name,
								crate::i18n::translate("stickers-message-sticker")
							)));
						}
						Provenance::Loading => {
							ui.add(line(crate::i18n::translate(
								"stickers-message-loading-sticker-details",
							)));
						}
						Provenance::OtherServer if failed => {
							retry_row(
								ui,
								crate::i18n::translate(
									"stickers-message-from-an-unavailable-server",
								),
								sticker.id,
								request,
							);
						}
						Provenance::OtherServer => {
							ui.add(line(crate::i18n::translate(
								"stickers-message-from-an-unavailable-server",
							)));
						}
						Provenance::Unavailable(error) => {
							retry_row(
								ui,
								error.map_or_else(
									|| {
										crate::i18n::translate(
											"stickers-message-sticker-details-unavailable",
										)
									},
									str::to_owned,
								),
								sticker.id,
								request,
							);
						}
					}
				});
			});
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
			if design::secondary_button(
				ui,
				&crate::i18n::translate("stickers-message-view-more-stickers"),
			)
			.clicked()
			{
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
	fn provenance_names_known_sources_and_explains_other_servers() {
		let mut item = sticker(7, 8);
		item.pack_id = None;
		assert_eq!(
			provenance(Some("Synthetic friends"), &item, true, None),
			Provenance::Named("Synthetic friends")
		);
		assert_eq!(provenance(None, &item, true, None), Provenance::Loading);
		assert_eq!(
			provenance(None, &item, false, Some("Offline")),
			Provenance::Unavailable(Some("Offline"))
		);
		assert_eq!(
			provenance(None, &item, false, None),
			Provenance::Unavailable(None)
		);
		item.guild_id = Some(Id(99));
		assert_eq!(
			provenance(None, &item, false, None),
			Provenance::OtherServer
		);
		// A failed reload keeps the details already known.
		assert_eq!(
			provenance(None, &item, false, Some("Offline")),
			Provenance::OtherServer
		);
	}
}
