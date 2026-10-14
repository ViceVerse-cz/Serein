//! Native embed cards. External links share the timeline's explicit confirmation.
use crate::{
	attachments::{DownloadUi, embed_context_menu},
	avatars::{Avatars, Surface},
	markdown::{FormatCache, external_url},
};
use egui::RichText;
use model::{Embed, Gif, Message};
use std::hash::{DefaultHasher, Hash, Hasher};

pub fn has_spoilers(message: &Message) -> bool {
	has_media_spoilers(message) || message.content.contains("||")
}
pub fn has_media_spoilers(message: &Message) -> bool {
	message.attachments.iter().any(|a| {
		a.spoiler
			|| a.filename.starts_with("SPOILER_")
			|| a.description.as_deref().is_some_and(|s| s.contains("||"))
	}) || message.embeds.iter().any(|e| {
		[&e.title, &e.description]
			.into_iter()
			.flatten()
			.any(|s| s.contains("||"))
			|| e.fields
				.iter()
				.any(|f| f.name.contains("||") || f.value.contains("||"))
			|| [&e.author, &e.provider]
				.into_iter()
				.flatten()
				.any(|a| a.name.contains("||"))
			|| e.footer.as_ref().is_some_and(|f| f.text.contains("||"))
	})
}
fn link(
	ui: &mut egui::Ui,
	label: &str,
	url: Option<&str>,
	opening: &mut Option<String>,
	strong: bool,
) {
	let target = url.and_then(external_url);
	let mut text = RichText::new(label);
	if strong {
		text = text.strong();
	}
	if target.is_some() {
		text = text.color(ui.visuals().hyperlink_color);
	}
	let response = ui.add(egui::Label::new(text).wrap().sense(if target.is_some() {
		egui::Sense::click()
	} else {
		egui::Sense::hover()
	}));
	if let Some(target) = target {
		response
			.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Link, ui.is_enabled(), label));
		if response
			.on_hover_text(crate::i18n::translate("embeds-link-open-link"))
			.clicked()
		{
			*opening = Some(target);
		}
	}
}

/// A source action is needed only when the card and message have no existing source link.
fn source_fallback(
	message: &Message,
	embed: &Embed,
	cache: &mut FormatCache,
	message_links_shown: bool,
) -> Option<String> {
	let inline = inline_image(embed).is_some();
	if (!inline
		&& embed
			.title
			.as_deref()
			.is_some_and(|title| !title.is_empty()))
		|| embed.video.is_some()
		|| matches!(embed.kind.as_str(), "video" | "gifv")
	{
		return None;
	}
	let url = embed.url.as_deref().and_then(external_url)?;
	if (message_links_shown && cache.get(message.id, &message.content).links.contains(&url))
		|| (!inline
			&& [&embed.author, &embed.provider]
				.into_iter()
				.flatten()
				.any(|author| {
					!author.name.is_empty()
						&& author.url.as_deref().and_then(external_url).as_ref() == Some(&url)
				})) {
		return None;
	}
	Some(url)
}
#[allow(clippy::too_many_arguments)]
fn text(
	ui: &mut egui::Ui,
	message: &Message,
	part: (u16, &str),
	cache: &mut FormatCache,
	opening: &mut Option<String>,
	profile: &mut crate::profiles::ProfileSession,
	card_surface: crate::design::MessageCardSurface,
	media: (
		&mut Avatars,
		bool,
		&[model::Guild],
		&crate::mentions::MentionSource<'_>,
	),
) {
	let (images, demo, guilds, source) = media;
	let formatted = cache.get_part(message.id, part.0, part.1);
	formatted.show_with_images(
		ui,
		opening,
		&message.mentions,
		Some(source),
		profile,
		(images, demo, guilds),
		card_surface,
	);
	if formatted.limited {
		ui.small(crate::i18n::translate("embeds-text-text-display-limited"));
	}
}
pub fn standalone_media_links(message: &Message) -> bool {
	!message.embeds_suppressed
		&& !has_spoilers(message)
		&& !message.content.trim().is_empty()
		&& message.content.split_whitespace().all(|link| {
			message.embeds.iter().any(|embed| {
				inline_image(embed).is_some()
					&& (embed.url.as_deref() == Some(link)
						|| [&embed.image, &embed.thumbnail]
							.into_iter()
							.flatten()
							.any(|media| media.url.as_deref() == Some(link)))
			})
		})
}

fn inline_image(embed: &Embed) -> Option<&model::EmbedMedia> {
	matches!(embed.kind.as_str(), "image" | "gifv")
		.then(|| {
			embed
				.image
				.as_ref()
				.or(embed.thumbnail.as_ref())
				.or_else(|| {
					(embed.kind == "gifv")
						.then_some(embed.video.as_ref())
						.flatten()
				})
		})
		.flatten()
}

// Discord link previews carry additional images as same-URL embed entries.
// Only consume continuations without independent content (or with repeated metadata).
fn gallery_len(embeds: &[Embed]) -> usize {
	let Some(first) = embeds.first() else {
		return 0;
	};
	let eligible = |e: &Embed| {
		e.image.is_some()
			&& e.video.is_none()
			&& matches!(e.kind.as_str(), "rich" | "article" | "link" | "image")
	};
	if !eligible(first) || first.url.as_deref().is_none_or(str::is_empty) {
		return 1;
	}
	1 + embeds[1..]
		.iter()
		.take_while(|e| {
			eligible(e)
				&& e.url == first.url
				&& (e.title.is_none() || e.title == first.title)
				&& (e.description.is_none() || e.description == first.description)
				&& (e.author.is_none() || e.author == first.author)
				&& (e.provider.is_none() || e.provider == first.provider)
				&& (e.footer.is_none() || e.footer == first.footer)
				&& (e.timestamp.is_none() || e.timestamp == first.timestamp)
				&& (e.thumbnail.is_none() || e.thumbnail == first.thumbnail)
				&& (e.fields.is_empty() || e.fields == first.fields)
		})
		.count()
}

fn gallery_rect(count: usize, index: usize, width: f32) -> egui::Rect {
	let gap = 4.0_f32.min(width / 4.0);
	if count >= 5 && width >= 300.0 {
		let side = (width - 2.0 * gap) / 3.0;
		return egui::Rect::from_min_size(
			egui::pos2(
				(index % 3) as f32 * (side + gap),
				(index / 3) as f32 * (side + gap),
			),
			egui::Vec2::splat(side),
		);
	}
	let half = (width - gap) / 2.0;
	let (x, y, height) = if count == 3 {
		if index == 0 {
			(0.0, 0.0, width)
		} else {
			(half + gap, (index - 1) as f32 * (half + gap), half)
		}
	} else {
		(
			(index % 2) as f32 * (half + gap),
			(index / 2) as f32 * (half + gap),
			half,
		)
	};
	egui::Rect::from_min_size(egui::pos2(x, y), egui::vec2(half, height))
}

fn gallery(
	ui: &mut egui::Ui,
	embeds: &[Embed],
	images: &mut Avatars,
	message: model::Id,
	download: &mut DownloadUi,
	demo: bool,
) {
	let width = ui
		.available_width()
		.clamp(1.0, crate::avatars::media::MEDIA_MAX_WIDTH);
	let height = gallery_rect(embeds.len(), embeds.len() - 1, width).bottom();
	let (area, _) = ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::hover());
	for (index, embed) in embeds.iter().enumerate() {
		let rect = gallery_rect(embeds.len(), index, width).translate(area.min.to_vec2());
		ui.scope_builder(
			egui::UiBuilder::new()
				.id_salt(("gallery", index))
				.max_rect(rect),
			|ui| {
				let media = embed.image.as_ref().expect("gallery has images");
				let target = media
					.url
					.as_deref()
					.or(media.proxy_url.as_deref())
					.and_then(external_url);
				let image = images
					.show_media(ui, media, rect.size(), demo, Surface::Banner)
					.response;
				let response =
					ui.interact(image.rect, image.id.with("media"), egui::Sense::click());
				embed_context_menu(&response, media, download, demo);
				let star =
					gif_for_media(media, embed.url.as_deref(), embed.kind == "gifv").map(|gif| {
						let favorite = download.gif_favorites.contains(&gif.url);
						let star = favorite_star(ui, &response, favorite);
						if star.clicked() {
							download.gif_favorite_request = Some(gif);
						}
						star
					});
				response.widget_info(|| {
					egui::WidgetInfo::labeled(
						if target.is_some() {
							egui::Role::Button
						} else {
							egui::Role::Image
						},
						ui.is_enabled(),
						format!(
							"{} {} {} {}",
							crate::i18n::translate("embeds-gallery-open-embed-image"),
							index + 1,
							crate::i18n::translate("embeds-gallery-of"),
							embeds.len()
						),
					)
				});
				if response.has_focus() {
					ui.painter().rect_stroke(
						rect,
						5,
						ui.visuals().selection.stroke,
						egui::StrokeKind::Inside,
					);
				}
				if target.is_some()
					&& !star
						.as_ref()
						.is_some_and(|star| star.hovered() || star.clicked())
					&& response
						.on_hover_text(crate::i18n::translate("embeds-gallery-open-image"))
						.clicked()
				{
					download.view_embed(message, media);
				}
			},
		);
	}
}

fn gif_for_embed(embed: &Embed, gifs: &client_core::gifs::Gifs) -> Option<Gif> {
	let media = [
		embed.video.as_ref().filter(|_| embed.kind == "gifv"),
		embed.image.as_ref(),
		embed.thumbnail.as_ref(),
	];
	let matches = |gif: &&Gif| {
		embed.url.as_deref() == Some(gif.url.as_str())
			|| media.iter().flatten().any(|image| {
				image
					.url
					.as_deref()
					.is_some_and(|url| url == gif.url || url == gif.preview)
			})
	};
	if let Some(gif) = gifs
		.favorites
		.iter()
		.find(matches)
		.or_else(|| gifs.view.as_ref()?.page.as_ref()?.gifs.iter().find(matches))
	{
		return Some(gif.clone());
	}
	media
		.iter()
		.flatten()
		.find_map(|media| gif_for_media(media, embed.url.as_deref(), embed.kind == "gifv"))
}

pub(crate) fn gif_for_media(
	media: &model::EmbedMedia,
	page: Option<&str>,
	animated: bool,
) -> Option<Gif> {
	let is_gif = |url: &str| {
		url.split('?')
			.next()
			.unwrap_or(url)
			.to_ascii_lowercase()
			.ends_with(".gif")
	};
	if !animated
		&& !media.url.as_deref().is_some_and(is_gif)
		&& !media.proxy_url.as_deref().is_some_and(is_gif)
	{
		return None;
	}
	let preview = [media.url.as_deref(), media.proxy_url.as_deref()]
		.into_iter()
		.flatten()
		.find(|url| model::valid_gif_favorite_source(url))?;
	let url = page
		.filter(|url| model::valid_gif_favorite_url(url))
		.or(media
			.url
			.as_deref()
			.filter(|url| model::valid_gif_favorite_url(url)))
		.unwrap_or(preview);
	let mut hash = DefaultHasher::new();
	url.hash(&mut hash);
	let gif = Gif {
		id: format!("chat-{:016x}", hash.finish()),
		title: String::new(),
		url: url.to_owned(),
		preview: preview.to_owned(),
		width: media.width.clamp(1, 4096),
		height: media.height.clamp(1, 4096),
	};
	gif.valid().then_some(gif)
}

/// Discord shows a sizable landscape link preview thumbnail as the card's large image.
/// Bot (`rich`) embeds keep their explicit side thumbnail.
fn large_thumbnail(embed: &Embed) -> Option<&model::EmbedMedia> {
	embed.thumbnail.as_ref().filter(|thumbnail| {
		embed.image.is_none()
			&& matches!(embed.kind.as_str(), "article" | "link" | "video")
			&& thumbnail.width >= 300
			&& thumbnail.width as f32 >= thumbnail.height as f32 * 1.2
	})
}

/// Discord's closed-poll card: the winning answer and a jump back to the poll.
/// Returns whether "View Poll" was clicked; `can_open` reflects whether the poll is reachable.
pub fn poll_result(ui: &mut egui::Ui, message: &Message, can_open: bool) -> bool {
	let Some(embed) = message.embeds.iter().find(|e| e.kind == "poll_result") else {
		return false;
	};
	let number = |name| {
		embed
			.field(name)
			.and_then(|value| value.trim().parse::<u64>().ok())
	};
	let total = number("total_votes").unwrap_or(0);
	let winner = embed
		.field("victor_answer_text")
		.map(str::trim)
		.filter(|text| !text.is_empty());
	let (title, detail) = match winner {
		Some(text) => {
			let votes = number("victor_answer_votes").unwrap_or(0).min(total);
			let percent = (votes * 100 + total / 2).checked_div(total).unwrap_or(0);
			// Custom emoji images are not resolved here; Unicode emoji carry no id.
			let emoji = embed
				.field("victor_answer_emoji_name")
				.filter(|_| embed.field("victor_answer_emoji_id").is_none());
			(
				emoji.map_or_else(|| text.to_owned(), |emoji| format!("{emoji} {text}")),
				Some(format!(
					"{} • {percent}%",
					crate::i18n::translate("embeds-poll-result-winning-answer")
				)),
			)
		}
		None if total > 0 => (crate::i18n::translate("embeds-poll-result-tie"), None),
		None => (crate::i18n::translate("embeds-poll-result-no-votes"), None),
	};
	let colors = crate::design::palette(ui);
	let width = ui.available_width().min(440.0);
	let mut clicked = false;
	egui::Frame::new()
		.fill(crate::design::message_card_fill(ui, colors.raised))
		.stroke(crate::design::message_card_stroke(ui))
		.corner_radius(8)
		.inner_margin(egui::Margin::symmetric(16, 10))
		.show(ui, |ui| {
			ui.set_width((width - 34.0).max(1.0));
			ui.allocate_ui_with_layout(
				egui::vec2(ui.available_width(), crate::design::BUTTON_HEIGHT + 4.0),
				egui::Layout::right_to_left(egui::Align::Center),
				|ui| {
					clicked = ui
						.add_enabled_ui(can_open, |ui| {
							crate::design::button(
								ui,
								"embeds-poll-result-view-poll",
								crate::design::ButtonKind::Outline,
							)
						})
						.inner
						.clicked();
					ui.add_space(8.0);
					ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| {
						ui.spacing_mut().item_spacing.y = 2.0;
						if detail.is_none() {
							ui.add_space(10.0);
						}
						ui.horizontal(|ui| {
							ui.spacing_mut().item_spacing.x = 6.0;
							let badge = if winner.is_some() { 24.0 } else { 0.0 };
							ui.scope(|ui| {
								ui.set_max_width((ui.available_width() - badge).max(1.0));
								ui.add(
									egui::Label::new(
										crate::design::medium(ui, &title, 15.0)
											.color(colors.text_strong),
									)
									.truncate(),
								);
							});
							if winner.is_some() {
								let (rect, _) = ui.allocate_exact_size(
									egui::Vec2::splat(18.0),
									egui::Sense::hover(),
								);
								ui.painter()
									.circle_filled(rect.center(), 9.0, colors.positive);
								crate::icons::paint(
									ui.painter(),
									crate::icons::Icon::Check,
									rect.shrink(4.0),
									egui::Color32::WHITE,
								);
							}
						});
						if let Some(detail) = &detail {
							ui.label(RichText::new(detail).size(13.0).color(colors.muted));
						}
					});
				},
			);
		});
	ui.add_space(6.0);
	clicked && can_open
}

fn image_preview(
	ui: &mut egui::Ui,
	message: model::Id,
	image: &model::EmbedMedia,
	size: egui::Vec2,
	images: &mut Avatars,
	download: &mut DownloadUi,
	demo: bool,
) {
	let painted = images
		.show_media(ui, image, size, demo, Surface::Inline)
		.response;
	let response = ui.interact(painted.rect, painted.id.with("media"), egui::Sense::click());
	response.widget_info(|| {
		egui::WidgetInfo::labeled(
			egui::Role::Button,
			ui.is_enabled(),
			crate::i18n::translate("embeds-image-preview-image-actions"),
		)
	});
	embed_context_menu(&response, image, download, demo);
	if response.clicked() {
		download.view_embed(message, image);
	}
}

#[allow(clippy::too_many_arguments)]
pub fn show(
	ui: &mut egui::Ui,
	message: &Message,
	cache: &mut FormatCache,
	images: &mut Avatars,
	opening: &mut Option<String>,
	download: &mut DownloadUi,
	profile: &mut crate::profiles::ProfileSession,
	state: &client_core::State,
	card_surface: crate::design::MessageCardSurface,
	hide_media_links: bool,
) -> Option<Gif> {
	if message.embeds_suppressed {
		return None;
	}
	let source = crate::mentions::MentionSource {
		state,
		channel: message.channel,
	};
	let demo = state.demo;
	let mut favorite_action = None;
	let mut index = 0;
	while index < message.embeds.len() {
		let count = gallery_len(&message.embeds[index..]);
		let group = &message.embeds[index..index + count];
		let embed = &group[0];
		// Poll results render as a dedicated card next to their system row.
		if embed.kind == "poll_result" {
			index += count;
			continue;
		}
		let source_url = source_fallback(
			message,
			embed,
			cache,
			!hide_media_links || !standalone_media_links(message),
		);
		ui.push_id(("embed", index), |ui| {
			if count > 1 && inline_image(embed).is_some() {
				gallery(ui, group, images, message.id, download, demo);
				if let Some(url) = &source_url {
					link(
						ui,
						&crate::i18n::translate("embeds-link-open-link"),
						Some(url),
						opening,
						false,
					);
				}
				if group.iter().any(|e| e.limited) {
					ui.small(crate::i18n::translate("embeds-show-embed-display-limited"));
				}
				ui.add_space(6.0);
				return;
			}
			if let Some(image) = inline_image(embed) {
				let gif = gif_for_embed(embed, &state.gifs);
				let painted = images.show_gif_embed(
					ui,
					embed,
					gif.as_ref(),
					egui::vec2(
						ui.available_width()
							.min(crate::avatars::media::MEDIA_MAX_WIDTH),
						crate::avatars::media::MEDIA_MAX_HEIGHT,
					),
					demo,
				);
				let response =
					ui.interact(painted.rect, painted.id.with("media"), egui::Sense::click());
				response.widget_info(|| {
					egui::WidgetInfo::labeled(
						egui::Role::Button,
						ui.is_enabled(),
						crate::i18n::translate("embeds-show-open-image"),
					)
				});
				embed_context_menu(&response, image, download, demo);
				let star = gif.map(|gif| {
					let favorite = state.is_gif_favorite(&gif);
					let star = favorite_star(ui, &response, favorite);
					if star.clicked() {
						favorite_action = Some(gif);
					}
					star
				});
				if !star
					.as_ref()
					.is_some_and(|star| star.hovered() || star.clicked())
					&& response
						.on_hover_text(crate::i18n::translate("embeds-show-open-image-2"))
						.clicked()
				{
					download.view_embed(message.id, image);
				}
				if let Some(url) = &source_url {
					link(
						ui,
						&crate::i18n::translate("embeds-link-open-link"),
						Some(url),
						opening,
						false,
					);
				}
				ui.add_space(6.0);
				return;
			}
			let colors = crate::design::palette(ui);
			let color = embed.color.map_or(colors.accent, |c| {
				egui::Color32::from_rgb((c >> 16) as u8, (c >> 8) as u8, c as u8)
			});
			let width = ui.available_width().min(480.0);
			let frame = egui::Frame::new()
				.fill(card_surface.fill(ui, colors.raised))
				.stroke(crate::design::message_card_stroke(ui))
				.corner_radius(5)
				.inner_margin(12)
				.show(ui, |ui| {
					ui.set_width((width - 24.0).max(1.0));
					// Size independently of the remaining timeline viewport.
					ui.set_max_height(640.0);
					// Bound exceptional cards' geometry; normal cards grow to their content height.
					egui::ScrollArea::vertical()
						.id_salt("embed-content")
						.max_height(640.0)
						.auto_shrink([false, true])
						.show(ui, |ui| {
							let part = 1 + index as u16 * 64;
							let large = large_thumbnail(embed).filter(|_| count == 1);
							let thumbnail = embed
								.thumbnail
								.as_ref()
								.filter(|_| large.is_none() && ui.available_width() >= 300.0);
							let body_width = (ui.available_width()
								- if thumbnail.is_some() { 96.0 } else { 0.0 })
							.max(1.0);
							ui.horizontal_top(|ui| {
								ui.vertical(|ui| {
									ui.set_width(body_width);
									if let Some(provider) = &embed.provider {
										link(
											ui,
											&provider.name,
											provider.url.as_deref(),
											opening,
											false,
										);
									}
									if let Some(author) = &embed.author {
										// Discord: a 24 px icon and the name in strong text,
										// a link only on hover.
										ui.horizontal_wrapped(|ui| {
											ui.spacing_mut().item_spacing.x = 8.0;
											if let Some(icon) = &author.icon {
												images.show_media(
													ui,
													icon,
													egui::vec2(24.0, 24.0),
													demo,
													Surface::Inline,
												);
											}
											author_name(ui, author, opening, colors.text_strong);
										});
										ui.add_space(2.0);
									}
									if let Some(title) = &embed.title {
										link(ui, title, embed.url.as_deref(), opening, true);
									}
									if let Some(description) = &embed.description {
										text(
											ui,
											message,
											(part, description),
											cache,
											opening,
											profile,
											card_surface,
											(images, demo, &state.guilds, &source),
										);
									}
								});
								if let Some(image) = thumbnail {
									image_preview(
										ui,
										message.id,
										image,
										egui::vec2(84.0, 84.0),
										images,
										download,
										demo,
									);
								}
							});
							let mut field = 0;
							while field < embed.fields.len() {
								let columns = if ui.available_width() >= 360.0 {
									3
								} else if ui.available_width() >= 240.0 {
									2
								} else {
									1
								};
								let count = if embed.fields[field].inline {
									embed.fields[field..]
										.iter()
										.take_while(|f| f.inline)
										.take(columns)
										.count()
								} else {
									1
								};
								ui.columns(count, |columns| {
									for (offset, column) in columns.iter_mut().enumerate() {
										let f = &embed.fields[field + offset];
										// Each field gets its own id namespace: unlike the cache
										// key above, egui's auto-assigned widget ids aren't
										// namespaced by field index, so adjacent fields can
										// otherwise collide and clash (visible as egui's debug
										// "used the same ID" warning in debug builds).
										column.push_id(field + offset, |column| {
											column.add(
												egui::Label::new(RichText::new(&f.name).strong())
													.wrap()
													.selectable(true),
											);
											text(
												column,
												message,
												(part + 1 + (field + offset) as u16, &f.value),
												cache,
												opening,
												profile,
												card_surface,
												(images, demo, &state.guilds, &source),
											);
										});
									}
								});
								field += count;
							}
							if count > 1 {
								gallery(ui, group, images, message.id, download, demo);
							} else if let Some(image) = embed.image.as_ref().or(large) {
								image_preview(
									ui,
									message.id,
									image,
									egui::vec2(
										ui.available_width(),
										crate::avatars::media::MEDIA_MAX_HEIGHT,
									),
									images,
									download,
									demo,
								);
							}
							if thumbnail.is_none()
								&& large.is_none() && let Some(image) = &embed.thumbnail
							{
								image_preview(
									ui,
									message.id,
									image,
									egui::vec2(84.0, 84.0),
									images,
									download,
									demo,
								);
							}
							if embed.video.is_some()
								|| matches!(embed.kind.as_str(), "video" | "gifv")
							{
								ui.small(crate::i18n::translate(
									"embeds-show-video-preview-playback-opens-in-your-browser",
								));
								link(
									ui,
									"Open video…",
									embed.url.as_deref().or_else(|| {
										embed.video.as_ref().and_then(|v| v.url.as_deref())
									}),
									opening,
									false,
								);
							}
							// One footer line, as on Discord: icon, text, then " • " and the
							// embed time in the chosen clock style.
							let when = embed.timestamp.as_deref().map(|raw| {
								time::OffsetDateTime::parse(
									raw,
									&time::format_description::well_known::Rfc3339,
								)
								.map_or_else(|_| raw.to_owned(), crate::local_time::calendar)
							});
							let footer_text = embed
								.footer
								.as_ref()
								.map(|footer| footer.text.as_str())
								.filter(|text| !text.is_empty());
							if footer_text.is_some() || when.is_some() {
								ui.add_space(4.0);
								ui.horizontal_wrapped(|ui| {
									ui.spacing_mut().item_spacing.x = 8.0;
									if let Some(icon) =
										embed.footer.as_ref().and_then(|f| f.icon.as_ref())
									{
										images.show_media(
											ui,
											icon,
											egui::vec2(20.0, 20.0),
											demo,
											Surface::Inline,
										);
									}
									let line = match (footer_text, when.as_deref()) {
										(Some(text), Some(when)) => format!("{text} • {when}"),
										(Some(text), None) => text.to_owned(),
										(None, Some(when)) => when.to_owned(),
										(None, None) => String::new(),
									};
									let mut text =
										RichText::new(line).size(12.0).color(colors.muted);
									if source_url.is_some() {
										text = text.underline();
									}
									let response = ui.add(
										egui::Label::new(text)
											.wrap()
											.selectable(source_url.is_none())
											.sense(if source_url.is_some() {
												egui::Sense::click()
											} else {
												egui::Sense::hover()
											}),
									);
									if let Some(url) = &source_url {
										response.widget_info(|| {
											egui::WidgetInfo::labeled(
												egui::Role::Link,
												ui.is_enabled(),
												url,
											)
										});
										if response
											.on_hover_text(url)
											.on_hover_cursor(egui::CursorIcon::PointingHand)
											.clicked()
										{
											*opening = Some(url.clone());
										}
									}
								});
							} else if let Some(url) = &source_url {
								link(
									ui,
									&crate::i18n::translate("embeds-link-open-link"),
									Some(url),
									opening,
									false,
								);
							}
							if group.iter().any(|e| e.limited) {
								ui.small(crate::i18n::translate(
									"embeds-show-embed-display-limited",
								));
							}
							if !matches!(
								embed.kind.as_str(),
								"rich" | "article" | "link" | "image" | "video" | "gifv"
							) {
								ui.small(crate::i18n::translate(
									"embeds-show-additional-embed-content-is-not-supported",
								));
							}
						});
				});
			ui.painter().line_segment(
				[
					frame.response.rect.left_top() + egui::vec2(1.5, 5.0),
					frame.response.rect.left_bottom() - egui::vec2(-1.5, 5.0),
				],
				egui::Stroke::new(3.0, color),
			);
			ui.add_space(6.0);
		});
		index += count;
	}
	favorite_action
}
pub fn estimated_height(embeds: &[Embed]) -> f32 {
	let mut height = 0.0;
	let mut index = 0;
	while index < embeds.len() {
		let count = gallery_len(&embeds[index..]);
		let e = &embeds[index];
		let image_height = if count > 1 {
			gallery_rect(count, count - 1, 456.0).bottom()
		} else if e.image.is_some() || large_thumbnail(e).is_some() {
			200.0
		} else {
			0.0
		};
		height += if e.kind == "poll_result" {
			72.0
		} else if inline_image(e).is_some() {
			let source = if e.kind == "image" && e.url.is_some() {
				20.0
			} else {
				0.0
			};
			(if count > 1 { image_height + 6.0 } else { 206.0 }) + source
		} else {
			let mut lines = 0.0;
			if e.provider
				.as_ref()
				.is_some_and(|provider| !provider.name.is_empty())
			{
				lines += 1.0;
			}
			if e.author.is_some() {
				lines += 1.0;
			}
			if e.title.is_some() {
				lines += 1.0;
			}
			if let Some(description) = e.description.as_deref().filter(|text| !text.is_empty()) {
				lines += description
					.lines()
					.map(|line| (line.chars().count() as f32 / 48.0).ceil().max(1.0))
					.sum::<f32>();
			}
			if e.footer
				.as_ref()
				.is_some_and(|footer| !footer.text.is_empty())
				|| e.timestamp.is_some()
				|| (e.url.is_some()
					&& e.title.as_deref().is_none_or(str::is_empty)
					&& e.video.is_none()
					&& !matches!(e.kind.as_str(), "video" | "gifv"))
			{
				lines += 1.0;
			}
			if lines == 0.0 {
				lines = 1.0;
			}
			let text = 24.0 + lines * 20.0 + 6.0;
			let thumb = if e.thumbnail.is_some() && large_thumbnail(e).is_none() {
				114.0
			} else {
				0.0
			};
			(text.max(thumb) + e.fields.len() as f32 * 44.0 + image_height).min(664.0)
		};
		index += count;
	}
	height
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn source_fallback_requires_a_safe_url_without_an_existing_source_action() {
		let mut message = test_support::message(1, model::Id(2));
		let mut embed = Embed {
			kind: "rich".into(),
			url: Some("https://example.org/post/1".into()),
			..Default::default()
		};
		let mut cache = FormatCache::default();
		assert_eq!(
			source_fallback(&message, &embed, &mut cache, true),
			embed.url
		);
		for content in [
			"https://example.org/post/1",
			"[Read the post](https://example.org/post/1)",
		] {
			message.content = content.into();
			assert_eq!(source_fallback(&message, &embed, &mut cache, true), None);
		}
		for content in [
			"https://example.org/post/12",
			"`https://example.org/post/1`",
		] {
			message.content = content.into();
			assert_eq!(
				source_fallback(&message, &embed, &mut cache, true),
				embed.url
			);
		}
		embed.title = Some("A linked title".into());
		assert_eq!(source_fallback(&message, &embed, &mut cache, true), None);
		embed.title = Some(String::new());
		embed.author = Some(model::EmbedAuthor {
			name: "Author".into(),
			url: embed.url.clone(),
			..Default::default()
		});
		assert_eq!(source_fallback(&message, &embed, &mut cache, true), None);
		embed.provider = embed.author.take();
		assert_eq!(source_fallback(&message, &embed, &mut cache, true), None);
		embed.provider = None;
		embed.kind = "video".into();
		assert_eq!(source_fallback(&message, &embed, &mut cache, true), None);
		embed.kind = "rich".into();
		embed.video = Some(model::EmbedMedia::default());
		assert_eq!(source_fallback(&message, &embed, &mut cache, true), None);
		embed.video = None;
		for url in [
			None,
			Some("javascript:alert(1)"),
			Some("https://user:password@example.org/post/1"),
		] {
			embed.url = url.map(str::to_owned);
			assert_eq!(source_fallback(&message, &embed, &mut cache, true), None);
		}
	}

	#[test]
	fn untitled_card_source_author_and_provider_dispatch_their_own_urls() {
		fn text_center(shape: &egui::Shape, label: &str) -> Option<egui::Pos2> {
			match shape {
				egui::Shape::Text(text) if text.galley.job.text == label => {
					Some(text.pos + text.galley.rect.center().to_vec2())
				}
				egui::Shape::Vec(shapes) => {
					shapes.iter().find_map(|shape| text_center(shape, label))
				}
				_ => None,
			}
		}
		for footer in [true, false] {
			let source_label = crate::i18n::translate("embeds-link-open-link");
			for (label, url) in [
				(
					if footer { "X" } else { &source_label },
					"https://example.org/post/1",
				),
				("Author", "https://example.org/author"),
				("Provider", "https://example.org/provider"),
			] {
				let mut message = test_support::message(1, model::Id(2));
				message.content = "A post without its URL in the message".into();
				message.embeds = vec![Embed {
					kind: "rich".into(),
					url: Some("https://example.org/post/1".into()),
					author: Some(model::EmbedAuthor {
						name: "Author".into(),
						url: Some("https://example.org/author".into()),
						..Default::default()
					}),
					provider: Some(model::EmbedAuthor {
						name: "Provider".into(),
						url: Some("https://example.org/provider".into()),
						..Default::default()
					}),
					footer: footer.then(|| model::EmbedFooter {
						text: "X".into(),
						..Default::default()
					}),
					..Default::default()
				}];
				let ctx = egui::Context::default();
				let mut opening = None;
				let mut cache = FormatCache::default();
				let mut images = Avatars::default();
				let mut frame = |events| {
					ctx.run_ui(
						egui::RawInput {
							screen_rect: Some(egui::Rect::from_min_size(
								egui::Pos2::ZERO,
								egui::vec2(640.0, 700.0),
							)),
							events,
							..Default::default()
						},
						|ui| {
							show(
								ui,
								&message,
								&mut cache,
								&mut images,
								&mut opening,
								&mut DownloadUi::default(),
								&mut crate::profiles::ProfileSession::default(),
								&client_core::State::default(),
								crate::design::MessageCardSurface::Conversation,
								false,
							);
						},
					)
				};
				frame(vec![]).drop_without_applying_deltas();
				let output = frame(vec![]);
				let pos = output
					.shapes
					.iter()
					.find_map(|shape| text_center(&shape.shape, label))
					.expect(label);
				output.drop_without_applying_deltas();
				for pressed in [true, false] {
					frame(vec![
						egui::Event::PointerMoved(pos),
						egui::Event::PointerButton {
							pos,
							button: egui::PointerButton::Primary,
							pressed,
							modifiers: egui::Modifiers::NONE,
						},
					])
					.drop_without_applying_deltas();
				}
				assert_eq!(opening.as_deref(), Some(url), "{label}, footer={footer}");
			}
		}
	}

	#[test]
	fn image_previews_dispatch_copy_and_save_without_opening_links() {
		for variant in 0..5 {
			for (copy, hide_media_links) in
				[(true, false), (false, false), (true, true), (false, true)]
			{
				let mut message = test_support::message(1, model::Id(2));
				let media = model::EmbedMedia {
					url: Some("https://cdn.discordapp.com/attachments/2/42/preview.png".into()),
					width: 160,
					height: 90,
					..Default::default()
				};
				message.embeds = vec![Embed {
					kind: match variant {
						0 | 4 => "image",
						1 => "gifv",
						_ => "rich",
					}
					.into(),
					url: Some("https://example.org/post".into()),
					image: (variant != 3).then(|| media.clone()),
					thumbnail: (variant == 3).then(|| media.clone()),
					..Default::default()
				}];
				if hide_media_links {
					message.content = message.embeds[0].url.clone().unwrap();
				}
				if variant == 4 {
					message.embeds.push(message.embeds[0].clone());
					message.embeds[0].image.as_mut().unwrap().url =
						Some("https://cdn.discordapp.com/attachments/2/43/other.png".into());
				}
				let ctx = egui::Context::default();
				let mut images = Avatars::default();
				let mut cache = FormatCache::default();
				let mut opening = None;
				let mut download = DownloadUi::default();
				let mut frame = |events| {
					ctx.run_ui(
						egui::RawInput {
							screen_rect: Some(egui::Rect::from_min_size(
								egui::Pos2::ZERO,
								egui::vec2(640.0, 700.0),
							)),
							events,
							..Default::default()
						},
						|ui| {
							let mut profile = crate::profiles::ProfileSession::default();
							assert!(
								show(
									ui,
									&message,
									&mut cache,
									&mut images,
									&mut opening,
									&mut download,
									&mut profile,
									&client_core::State::default(),
									crate::design::MessageCardSurface::Conversation,
									hide_media_links,
								)
								.is_none()
							);
						},
					)
				};
				frame(vec![]).drop_without_applying_deltas();
				let output = frame(vec![]);
				let pos = output
					.shapes
					.iter()
					.filter_map(|shape| match &shape.shape {
						egui::Shape::Rect(shape)
							if shape.corner_radius == egui::CornerRadius::same(5)
								&& shape.rect.width() > 40.0
								&& shape.rect.height() > 30.0 =>
						{
							Some(shape.rect.center())
						}
						_ => None,
					})
					.next_back()
					.expect("rendered image");
				assert_eq!(
					output.shapes.iter().any(|shape| matches!(&shape.shape,
						egui::Shape::Text(text) if text.galley.job.text == crate::i18n::translate("embeds-link-open-link")
					)),
					variant != 1 && (!hide_media_links || matches!(variant, 0 | 4)),
					"source action for media variant {variant}"
				);
				output.drop_without_applying_deltas();
				for pressed in [true, false] {
					frame(vec![
						egui::Event::PointerMoved(pos),
						egui::Event::PointerButton {
							pos,
							button: egui::PointerButton::Secondary,
							pressed,
							modifiers: egui::Modifiers::NONE,
						},
					])
					.drop_without_applying_deltas();
				}
				let output = frame(vec![]);
				let label = if copy {
					"Copy image"
				} else {
					"Save image as…"
				};
				let target = output
					.shapes
					.iter()
					.find_map(|shape| match &shape.shape {
						egui::Shape::Text(text) if text.galley.job.text == label => {
							Some(text.pos + text.galley.rect.center().to_vec2())
						}
						_ => None,
					})
					.unwrap_or_else(|| panic!("missing {label} for embed variant {variant}"));
				output.drop_without_applying_deltas();
				for pressed in [true, false] {
					frame(vec![
						egui::Event::PointerMoved(target),
						egui::Event::PointerButton {
							pos: target,
							button: egui::PointerButton::Primary,
							pressed,
							modifiers: egui::Modifiers::NONE,
						},
					])
					.drop_without_applying_deltas();
				}
				assert_eq!(download.embed_request, Some((media, copy)));
				assert!(
					download.request.is_none()
						&& download.copy_request.is_none()
						&& opening.is_none()
				);
			}
		}
	}

	fn gallery_embeds(count: usize) -> Vec<Embed> {
		(0..count)
			.map(|index| Embed {
				kind: "rich".into(),
				url: Some("https://example.com/gallery".into()),
				image: Some(model::EmbedMedia {
					url: Some(format!(
						"https://cdn.discordapp.com/attachments/1/2/{index}.png"
					)),
					width: 640,
					height: 360,
					..Default::default()
				}),
				..Default::default()
			})
			.collect()
	}

	#[test]
	fn gallery_groups_only_related_images_and_preserves_independent_content() {
		let mut embeds = gallery_embeds(3);
		embeds[0].title = Some("Gallery title".into());
		assert_eq!(gallery_len(&embeds), 3);
		assert!(estimated_height(&embeds) < 3.0 * 300.0);
		embeds[1].title = embeds[0].title.clone();
		assert_eq!(gallery_len(&embeds), 3);
		for variant in 0..7 {
			let mut separate = embeds.clone();
			match variant {
				0 => separate[1].url = Some("https://example.com/other".into()),
				1 => separate.iter_mut().for_each(|e| e.url = None),
				2 => separate
					.iter_mut()
					.for_each(|e| e.url = Some(String::new())),
				3 => separate[1].title = Some("Independent title".into()),
				4 => separate[1].image = None,
				5 => separate[1].video = Some(Default::default()),
				_ => separate[1].kind = "gifv".into(),
			}
			assert_eq!(gallery_len(&separate), 1, "variant {variant}");
		}
		assert_eq!(gallery_len(&[]), 0);
		assert_eq!(
			gallery_len(&gallery_embeds(model::MAX_EMBEDS)),
			model::MAX_EMBEDS
		);
	}

	#[test]
	fn gallery_tiles_fit_without_overlap_and_request_each_image_viewer() {
		for theme in [egui::Theme::Dark, egui::Theme::Light] {
			for width in [96.0, 240.0, 456.0] {
				for count in [2, 3, 4, 10] {
					let rects: Vec<_> = (0..count).map(|i| gallery_rect(count, i, width)).collect();
					for (i, rect) in rects.iter().enumerate() {
						assert!(rect.width() > 0.0 && rect.right() <= width);
						assert!(rects[..i].iter().all(|other| !rect.intersects(*other)));
					}
					if count == 3 {
						assert_eq!(rects[0].height(), width);
						assert_eq!(rects[0].bottom(), rects[2].bottom());
						assert_eq!(rects[1].left(), rects[2].left());
					}
					let ctx = egui::Context::default();
					ctx.set_theme(theme);
					let embeds = gallery_embeds(count);
					let mut images = Avatars::default();
					let mut download = DownloadUi::default();
					let mut origin = egui::Pos2::ZERO;
					let mut frame = |events| {
						ctx.run_ui(
							egui::RawInput {
								screen_rect: Some(egui::Rect::from_min_size(
									egui::Pos2::ZERO,
									egui::vec2(width + 16.0, 1400.0),
								)),
								events,
								..Default::default()
							},
							|ui| {
								ui.set_width(width);
								origin = ui.cursor().min;
								gallery(
									ui,
									&embeds,
									&mut images,
									model::Id(42),
									&mut download,
									false,
								);
							},
						)
						.drop_without_applying_deltas();
						(origin, download.embed_view_request.take())
					};
					frame(vec![]);
					let (origin, _) = frame(vec![]);
					for (i, rect) in rects.iter().enumerate() {
						let pos = rect.center() + origin.to_vec2();
						frame(vec![
							egui::Event::PointerMoved(pos),
							egui::Event::PointerButton {
								pos,
								button: egui::PointerButton::Primary,
								pressed: true,
								modifiers: Default::default(),
							},
						]);
						let (_, opened) = frame(vec![egui::Event::PointerButton {
							pos,
							button: egui::PointerButton::Primary,
							pressed: false,
							modifiers: Default::default(),
						}]);
						assert_eq!(
							opened,
							Some((model::Id(42), embeds[i].image.as_ref().unwrap().clone()))
						);
					}
					assert_eq!(images.take_requests().len(), count);
				}
			}
		}
	}

	#[test]
	fn hide_media_links_requires_matching_visible_media_without_caption_or_spoiler() {
		let mut message = test_support::message(1, model::Id(1));
		message.content = "https://klipy.com/gifs/waving-lizard".into();
		message.embeds = vec![Embed {
			kind: "gifv".into(),
			url: Some(message.content.clone()),
			thumbnail: Some(model::EmbedMedia::default()),
			..Default::default()
		}];
		assert!(standalone_media_links(&message));
		message.embeds_suppressed = true;
		assert!(!standalone_media_links(&message));
		message.embeds_suppressed = false;
		message.content.insert_str(0, "Hello! ");
		assert!(!standalone_media_links(&message));
		message.content = "https://example.com/other.gif".into();
		assert!(!standalone_media_links(&message));
		message.content = message.embeds[0].url.clone().unwrap();
		message.embeds[0].kind = "rich".into();
		assert!(!standalone_media_links(&message));
	}

	#[test]
	fn text_spoilers_do_not_hide_ordinary_media_but_keep_reply_previews_conservative() {
		let mut message = test_support::message(1, model::Id(1));
		message.embeds = vec![Embed::default()];
		message.attachments = vec![model::Attachment {
			duration_ms: None,
			waveform: Vec::new(),
			id: model::Id(2),
			filename: "photo.png".into(),
			description: None,
			content_type: Some("image/png".into()),
			size: 1,
			media: Default::default(),
			spoiler: false,
		}];
		message.content = "Ordinary text".into();
		assert!(!has_spoilers(&message));
		message.content = "Ordinary text ||hidden text||".into();
		assert!(!has_media_spoilers(&message));
		assert!(has_spoilers(&message));
		message.content.clear();
		for field in 0..10 {
			let mut guarded = message.clone();
			let embed = &mut guarded.embeds[0];
			match field {
				0 => guarded.attachments[0].spoiler = true,
				1 => guarded.attachments[0].filename = "SPOILER_photo.png".into(),
				2 => guarded.attachments[0].description = Some("||hidden||".into()),
				3 => embed.title = Some("||hidden||".into()),
				4 => embed.description = Some("||hidden||".into()),
				5 => embed.fields.push(model::EmbedField {
					name: "||hidden||".into(),
					..Default::default()
				}),
				6 => embed.fields.push(model::EmbedField {
					value: "||hidden||".into(),
					..Default::default()
				}),
				7 => {
					embed.author = Some(model::EmbedAuthor {
						name: "||hidden||".into(),
						..Default::default()
					})
				}
				8 => {
					embed.provider = Some(model::EmbedAuthor {
						name: "||hidden||".into(),
						..Default::default()
					})
				}
				_ => {
					embed.footer = Some(model::EmbedFooter {
						text: "||hidden||".into(),
						..Default::default()
					})
				}
			}
			assert!(has_media_spoilers(&guarded), "media safety field {field}");
			assert!(has_spoilers(&guarded), "reply safety field {field}");
		}
	}

	/// Adjacent fields render through the same field-column code path with no explicit
	/// id_salt of their own, so egui's auto-assigned widget ids can land on the same value
	/// for two different fields (data-dependent on their wrapped line counts) unless each
	/// field is given its own id namespace. Regression for that: egui paints a "used the
	/// same ID" debug warning (gated on `debug_assertions`, which test builds have on) when
	/// two same-frame widgets collide, so its absence here confirms the fields stay isolated.
	#[test]
	fn adjacent_fields_do_not_share_widget_ids() {
		let mut message = test_support::message(1, model::Id(2));
		message.embeds = vec![Embed {
			kind: "rich".into(),
			description: Some(
				"For tutorials & guides, please check out our [Help Center](https://example.org/help)"
					.into(),
			),
			fields: vec![
				model::EmbedField {
					name: "No Staff Applications".into(),
					value: "We are currently **not looking** for staff, please do not open tickets asking to join the team.".into(),
					inline: false,
				},
				model::EmbedField {
					name: "Second field".into(),
					value: "Some other body text that also wraps to more than one line at this width.".into(),
					inline: false,
				},
			],
			..Default::default()
		}];
		let ctx = egui::Context::default();
		let mut images = Avatars::default();
		let mut cache = FormatCache::default();
		let mut opening = None;
		let mut download = DownloadUi::default();
		let output = ctx.run_ui(
			egui::RawInput {
				screen_rect: Some(egui::Rect::from_min_size(
					egui::Pos2::ZERO,
					egui::vec2(640.0, 700.0),
				)),
				..Default::default()
			},
			|ui| {
				let mut profile = crate::profiles::ProfileSession::default();
				show(
					ui,
					&message,
					&mut cache,
					&mut images,
					&mut opening,
					&mut download,
					&mut profile,
					&client_core::State::default(),
					crate::design::MessageCardSurface::Conversation,
					false,
				);
			},
		);
		for shape in &output.shapes {
			if let egui::epaint::Shape::Text(text) = &shape.shape {
				assert!(
					!text.galley.job.text.contains("widget ID"),
					"egui reported a widget id clash: {:?}",
					text.galley.job.text
				);
			}
		}
		output.drop_without_applying_deltas();
	}

	/// An X/Twitter link card, shaped like the service's: author, text, footer and time.
	#[test]
	fn tweet_card_puts_footer_and_time_on_one_line_without_a_source_link() {
		let _clock = crate::local_time::pin_now(
			time::OffsetDateTime::parse(
				"2026-10-14T12:00:00Z",
				&time::format_description::well_known::Rfc3339,
			)
			.unwrap(),
		);
		let mut message = test_support::message(1, model::Id(2));
		message.content = "https://x.com/synthetic/status/1".into();
		message.embeds = vec![Embed {
			kind: "rich".into(),
			url: Some(message.content.clone()),
			description: Some("A synthetic post".into()),
			author: Some(model::EmbedAuthor {
				name: "Synthetic (@synthetic)".into(),
				url: Some("https://x.com/synthetic".into()),
				..Default::default()
			}),
			footer: Some(model::EmbedFooter {
				text: "X".into(),
				..Default::default()
			}),
			timestamp: Some("2026-10-12T17:37:03.721000+00:00".into()),
			..Default::default()
		}];
		let ctx = egui::Context::default();
		let output = ctx.run_ui(
			egui::RawInput {
				screen_rect: Some(egui::Rect::from_min_size(
					egui::Pos2::ZERO,
					egui::vec2(640.0, 700.0),
				)),
				..Default::default()
			},
			|ui| {
				show(
					ui,
					&message,
					&mut FormatCache::default(),
					&mut Avatars::default(),
					&mut None,
					&mut DownloadUi::default(),
					&mut crate::profiles::ProfileSession::default(),
					&client_core::State::default(),
					crate::design::MessageCardSurface::Conversation,
					false,
				);
			},
		);
		let mut painted = Vec::new();
		fn walk(shape: &egui::Shape, out: &mut Vec<String>) {
			match shape {
				egui::Shape::Text(text) => out.push(text.galley.job.text.clone()),
				egui::Shape::Vec(shapes) => shapes.iter().for_each(|shape| walk(shape, out)),
				_ => {}
			}
		}
		output
			.shapes
			.iter()
			.for_each(|shape| walk(&shape.shape, &mut painted));
		output.drop_without_applying_deltas();
		let footer = painted
			.iter()
			.find(|text| text.starts_with("X • "))
			.unwrap_or_else(|| panic!("{painted:?}"));
		// Tests pin UTC; the clock style is whatever the process last set.
		assert!(
			footer == "X • 10/12/26, 17:37" || footer == "X • 10/12/26, 5:37 PM",
			"{footer}"
		);
		assert!(painted.iter().any(|text| text == "Synthetic (@synthetic)"));
		assert!(!painted.iter().any(|text| text.contains("Open source")));
		assert!(!painted.iter().any(|text| text.contains("2026-10-12T")));
	}
}

/// Shared star overlay for message GIFs and uploaded GIF attachments.
pub(crate) fn favorite_star(
	ui: &mut egui::Ui,
	response: &egui::Response,
	favorite: bool,
) -> egui::Response {
	let star_rect = egui::Rect::from_min_size(
		response.rect.right_top() + egui::vec2(-34.0, 4.0),
		egui::Vec2::splat(30.0),
	);
	let star = ui.interact(
		star_rect,
		response.id.with("favorite"),
		egui::Sense::click(),
	);
	ui.painter()
		.rect_filled(star_rect, 6, egui::Color32::from_black_alpha(190));
	crate::icons::paint(
		ui.painter(),
		if favorite {
			crate::icons::Icon::StarFill
		} else {
			crate::icons::Icon::Star
		},
		star_rect.shrink(5.0),
		if favorite {
			crate::design::palette(ui).warning
		} else {
			egui::Color32::WHITE
		},
	);
	if star.has_focus() {
		ui.painter().rect_stroke(
			star_rect,
			6,
			ui.visuals().selection.stroke,
			egui::StrokeKind::Inside,
		);
	}
	star.widget_info(|| {
		egui::WidgetInfo::selected(
			egui::Role::CheckBox,
			ui.is_enabled(),
			favorite,
			crate::i18n::translate("embeds-show-favorite-gif"),
		)
	});
	star.on_hover_text(crate::i18n::translate_if_key(
		&(if favorite {
			crate::i18n::translate("embeds-show-remove-from-gif-favorites")
		} else {
			crate::i18n::translate("embeds-show-save-to-gif-favorites")
		}),
	))
}

/// An embed author name: strong text that underlines on hover and opens its page if it has one.
fn author_name(
	ui: &mut egui::Ui,
	author: &model::EmbedAuthor,
	opening: &mut Option<String>,
	color: egui::Color32,
) {
	let url = author.url.as_deref().and_then(external_url);
	let response = ui.add(
		egui::Label::new(RichText::new(&author.name).size(14.0).strong().color(color))
			.wrap()
			.sense(if url.is_some() {
				egui::Sense::click()
			} else {
				egui::Sense::hover()
			}),
	);
	if let Some(url) = url {
		response.widget_info(|| {
			egui::WidgetInfo::labeled(egui::Role::Link, ui.is_enabled(), &author.name)
		});
		if response.hovered() {
			ui.painter().hline(
				response.rect.x_range(),
				response.rect.bottom(),
				egui::Stroke::new(1.0, color),
			);
		}
		if response
			.on_hover_cursor(egui::CursorIcon::PointingHand)
			.clicked()
		{
			*opening = Some(url);
		}
	}
}
