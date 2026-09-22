//! Native embed cards. External links share the timeline's explicit confirmation.
use crate::{
	attachments::{DownloadUi, embed_context_menu},
	avatars::Avatars,
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
		if response.on_hover_text("Open link…").clicked() {
			*opening = Some(target);
		}
	}
}
fn text(
	ui: &mut egui::Ui,
	message: &Message,
	part: (u16, &str),
	cache: &mut FormatCache,
	opening: &mut Option<String>,
	profile: &mut crate::profiles::ProfileSession,
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
	);
	if formatted.limited {
		ui.small("Text display limited");
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
		.then(|| embed.image.as_ref().or(embed.thumbnail.as_ref()))
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
	opening: &mut Option<String>,
	download: &mut DownloadUi,
	demo: bool,
) {
	let width = ui.available_width().clamp(1.0, 480.0);
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
				let image = images.show_banner(ui, media, rect.size(), demo);
				let response =
					ui.interact(image.rect, image.id.with("media"), egui::Sense::click());
				embed_context_menu(&response, media, download, demo);
				response.widget_info(|| {
					egui::WidgetInfo::labeled(
						if target.is_some() {
							egui::Role::Button
						} else {
							egui::Role::Image
						},
						ui.is_enabled(),
						format!("Open embed image {} of {}", index + 1, embeds.len()),
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
				if let Some(target) = target
					&& response.on_hover_text("Open image…").clicked()
				{
					*opening = Some(target);
				}
			},
		);
	}
}

fn gif_for_embed(embed: &Embed, gifs: &client_core::gifs::Gifs) -> Option<Gif> {
	let media = [
		embed.image.as_ref(),
		embed.thumbnail.as_ref(),
		embed.video.as_ref(),
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
	let preview = media.iter().flatten().find(|image| {
		image.url.as_deref().is_some_and(|url| {
			model::valid_gif_preview(url) && (embed.kind == "gifv" || url.ends_with(".gif"))
		})
	})?;
	let url = embed
		.url
		.as_deref()
		.filter(|url| model::valid_gif_url(url))
		.or_else(|| {
			preview
				.url
				.as_deref()
				.filter(|url| model::valid_gif_url(url))
		})?;
	let mut hash = DefaultHasher::new();
	url.hash(&mut hash);
	let gif = Gif {
		id: format!("chat-{:016x}", hash.finish()),
		title: embed.title.clone().unwrap_or_default(),
		url: url.to_owned(),
		preview: preview.url.clone()?,
		width: preview.width,
		height: preview.height,
	};
	gif.valid().then_some(gif)
}

fn image_preview(
	ui: &mut egui::Ui,
	image: &model::EmbedMedia,
	size: egui::Vec2,
	images: &mut Avatars,
	download: &mut DownloadUi,
	demo: bool,
) {
	let painted = images.show_embed(ui, image, size, demo);
	let response = ui.interact(painted.rect, painted.id.with("media"), egui::Sense::click());
	response.widget_info(|| {
		egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), "Image actions")
	});
	embed_context_menu(&response, image, download, demo);
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
		ui.push_id(("embed", index), |ui| {
			if count > 1 && inline_image(embed).is_some() {
				gallery(ui, group, images, opening, download, demo);
				if group.iter().any(|e| e.limited) {
					ui.small("Embed display limited");
				}
				ui.add_space(6.0);
				return;
			}
			if let Some(image) = inline_image(embed) {
				let gif = gif_for_embed(embed, &state.gifs);
				let painted =
					images.show_gif_embed(ui, embed, gif.as_ref(), egui::vec2(480.0, 320.0), demo);
				let response =
					ui.interact(painted.rect, painted.id.with("media"), egui::Sense::click());
				response.widget_info(|| {
					egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), "Open image")
				});
				embed_context_menu(&response, image, download, demo);
				let star = gif.map(|gif| {
					let favorite = state.is_gif_favorite(&gif);
					let star_rect = egui::Rect::from_min_size(
						response.rect.right_top() + egui::vec2(-34.0, 4.0),
						egui::Vec2::splat(30.0),
					);
					let star = ui.interact(
						star_rect,
						ui.scope_id().with("favorite"),
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
							"Favorite GIF",
						)
					});
					if star.clicked() {
						favorite_action = Some(gif);
					}
					star.on_hover_text(if favorite {
						"Remove from GIF favorites"
					} else {
						"Save to GIF favorites"
					})
				});
				if !star
					.as_ref()
					.is_some_and(|star| star.hovered() || star.clicked())
					&& response.on_hover_text("Open image…").clicked()
				{
					*opening = embed
						.url
						.as_deref()
						.or(image.url.as_deref())
						.and_then(external_url);
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
				.fill(colors.raised)
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
							let thumbnail = embed
								.thumbnail
								.as_ref()
								.filter(|_| ui.available_width() >= 300.0);
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
										ui.horizontal_wrapped(|ui| {
											if let Some(icon) = &author.icon {
												images.show_embed(
													ui,
													icon,
													egui::vec2(20.0, 20.0),
													demo,
												);
											}
											link(
												ui,
												&author.name,
												author.url.as_deref(),
												opening,
												false,
											);
										});
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
											(images, demo, &state.guilds, &source),
										);
									}
								});
								if let Some(image) = thumbnail {
									image_preview(
										ui,
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
												(images, demo, &state.guilds, &source),
											);
										});
									}
								});
								field += count;
							}
							if count > 1 {
								gallery(ui, group, images, opening, download, demo);
							} else if let Some(image) = &embed.image {
								image_preview(
									ui,
									image,
									egui::vec2(ui.available_width(), 320.0),
									images,
									download,
									demo,
								);
							}
							if thumbnail.is_none()
								&& let Some(image) = &embed.thumbnail
							{
								image_preview(
									ui,
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
								ui.small("Video preview · playback opens in your browser");
								link(
									ui,
									"Open video…",
									embed.url.as_deref().or_else(|| {
										embed.video.as_ref().and_then(|v| v.url.as_deref())
									}),
									opening,
									false,
								);
							} else if embed.title.is_none()
								&& let Some(url) = embed.url.as_deref()
							{
								link(ui, "Open source…", Some(url), opening, false);
							}
							if let Some(footer) = &embed.footer {
								ui.horizontal_wrapped(|ui| {
									if let Some(icon) = &footer.icon {
										images.show_embed(ui, icon, egui::vec2(16.0, 16.0), demo);
									}
									ui.add(
										egui::Label::new(
											RichText::new(&footer.text).small().color(colors.muted),
										)
										.wrap()
										.selectable(true),
									);
								});
							}
							if let Some(timestamp) = &embed.timestamp {
								ui.small(timestamp);
							}
							if group.iter().any(|e| e.limited) {
								ui.small("Embed display limited");
							}
							if !matches!(
								embed.kind.as_str(),
								"rich" | "article" | "link" | "image" | "video" | "gifv"
							) {
								ui.small("Additional embed content is not supported");
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
		} else if e.image.is_some() {
			200.0
		} else {
			0.0
		};
		height += if inline_image(e).is_some() {
			if count > 1 { image_height + 6.0 } else { 206.0 }
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
			{
				lines += 1.0;
			}
			if lines == 0.0 {
				lines = 1.0;
			}
			let text = 24.0 + lines * 20.0 + 6.0;
			let thumb = if e.thumbnail.is_some() { 114.0 } else { 0.0 };
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
	fn image_previews_dispatch_copy_and_save_without_opening_links() {
		for variant in 0..5 {
			for copy in [true, false] {
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
									&client_core::State::default()
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
	fn gallery_card_shows_one_title_all_images_and_keeps_suppression() {
		let mut message = test_support::message(1, model::Id(20));
		message.embeds = gallery_embeds(3);
		message.embeds[0].title = Some("Shared card title".into());
		message.embeds[1].limited = true;
		for width in [240.0, 480.0] {
			for suppressed in [false, true] {
				message.embeds_suppressed = suppressed;
				let ctx = egui::Context::default();
				let mut images = Avatars::default();
				let mut cache = FormatCache::default();
				let mut output = ctx.run_ui(
					egui::RawInput {
						screen_rect: Some(egui::Rect::from_min_size(
							egui::Pos2::ZERO,
							egui::vec2(width, 900.0),
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
							&mut None,
							&mut DownloadUi::default(),
							&mut profile,
							&client_core::State::default(),
						);
					},
				);
				let labels: Vec<_> = output
					.shapes
					.iter()
					.filter_map(|shape| match &shape.shape {
						egui::Shape::Text(text) => Some(text.galley.job.text.as_str()),
						_ => None,
					})
					.collect();
				assert_eq!(
					labels
						.iter()
						.filter(|label| **label == "Shared card title")
						.count(),
					usize::from(!suppressed)
				);
				assert_eq!(labels.contains(&"Embed display limited"), !suppressed);
				assert_eq!(images.take_requests().len(), if suppressed { 0 } else { 3 });
				output.textures_delta.clear();
			}
		}
	}

	#[test]
	fn gallery_tiles_fit_without_overlap_and_open_each_original() {
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
					let mut opening = None;
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
									&mut opening,
									&mut DownloadUi::default(),
									false,
								);
							},
						)
						.drop_without_applying_deltas();
						(origin, opening.take())
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
						assert_eq!(opened, embeds[i].image.as_ref().unwrap().url);
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
	fn chat_gifs_reuse_favorites_and_reject_unapproved_media() {
		let mut embed = Embed {
			kind: "gifv".into(),
			url: Some("https://klipy.com/gifs/synthetic-wave".into()),
			thumbnail: Some(model::EmbedMedia {
				url: Some("https://static.klipy.com/synthetic/wave.gif".into()),
				width: 320,
				height: 180,
				..Default::default()
			}),
			..Default::default()
		};
		let mut gifs = client_core::gifs::Gifs::default();
		let mut gif = gif_for_embed(&embed, &gifs).unwrap();
		assert!(gif.valid());
		gif.id = "provider-id".into();
		gifs.favorites.push(gif.clone());
		assert_eq!(gif_for_embed(&embed, &gifs), Some(gif));
		gifs.favorites.clear();
		embed.thumbnail.as_mut().unwrap().url = Some("https://example.com/wave.gif".into());
		assert!(gif_for_embed(&embed, &gifs).is_none());
	}

	#[test]
	fn direct_images_and_gifs_use_media_instead_of_cards() {
		let mut embed = Embed {
			kind: "image".into(),
			thumbnail: Some(model::EmbedMedia::default()),
			..Default::default()
		};
		assert!(inline_image(&embed).is_some());
		embed.kind = "gifv".into();
		assert!(inline_image(&embed).is_some());
		embed.kind = "rich".into();
		assert!(inline_image(&embed).is_none());
		embed.kind = "image".into();
		embed.thumbnail = None;
		assert!(inline_image(&embed).is_none());
		embed.image = Some(model::EmbedMedia::default());
		assert!(inline_image(&embed).is_some());
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
}
