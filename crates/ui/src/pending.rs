//! Own messages waiting for Discord: dimmed like Discord's optimistic rows, with every
//! attachment shown and one upload strip for the whole batch.
use client_core::{Pending, State};
use egui::RichText;
use model::Delivery;

use crate::{attachments, design, icons};

/// One file of a pending batch; the texture is the composer thumbnail, never re-decoded here.
pub struct UploadFile {
	pub bytes: u64,
	pub preview: Option<egui::TextureHandle>,
}
pub struct Upload {
	pub nonce: String,
	pub files: Vec<UploadFile>,
	/// Bytes handed to HTTP over the whole batch; never a delivery receipt.
	pub progress: Option<(u64, u64)>,
}
impl Upload {
	pub fn bytes(&self) -> u64 {
		self.files.iter().map(|file| file.bytes).sum()
	}
}

/// Attachment tile width and overall card width follow the confirmed-message layout.
const MAX_WIDTH: f32 = 432.0;

pub fn show(
	ui: &mut egui::Ui,
	pending: &Pending,
	compact: bool,
	state: &State,
	media: (
		&mut crate::avatars::Avatars,
		&mut Option<String>,
		&mut crate::profiles::ProfileSession,
		&mut Option<model::Id>,
		&mut crate::markdown::FormatCache,
	),
	upload: Option<&Upload>,
	(restore, cancel): (&mut Option<String>, &mut bool),
) {
	let colors = design::palette(ui);
	let (avatars, opening, profile, channel, formats) = media;
	let upload = upload.filter(|upload| upload.nonce == pending.nonce);
	let sending = pending.delivery == Delivery::Sending;
	let artwork = pending.attachments.len() == 1
		&& attachments::artwork_edge(&pending.attachments[0]).is_some();
	let status = match pending.delivery {
		Delivery::Sending => "Sending…",
		Delivery::Ambiguous => "Delivery unknown",
		Delivery::Rejected => "Not sent",
		Delivery::Confirmed => "Sent",
	};
	egui::Frame::NONE
		.inner_margin(egui::Margin {
			left: 16,
			right: 16,
			top: if compact { 1 } else { 14 },
			bottom: 1,
		})
		.show(ui, |ui| {
			ui.spacing_mut().item_spacing = egui::vec2(16.0, 4.0);
			ui.horizontal_top(|ui| {
				if compact {
					ui.allocate_exact_size(
						egui::vec2(40.0, crate::timeline::MESSAGE_LINE),
						egui::Sense::hover(),
					);
				} else {
					ui.scope(|ui| {
						ui.set_opacity(0.55);
						if let Some(user) = &state.user {
							avatars.show(ui, user, 40.0, state.demo);
						} else {
							ui.allocate_exact_size(egui::vec2(40.0, 40.0), egui::Sense::hover());
						}
					});
				}
				ui.vertical(|ui| {
					ui.set_width(ui.available_width());
					let mut text_line = egui::Rect::NOTHING;
					if !compact
						|| matches!(pending.delivery, Delivery::Rejected | Delivery::Ambiguous)
					{
						ui.horizontal_wrapped(|ui| {
							ui.spacing_mut().item_spacing.x = 8.0;
							if !compact {
								ui.label(
									design::medium(
										ui,
										state.user.as_ref().map_or("You", |u| &u.name),
										15.5,
									)
									.color(colors.muted),
								);
							}
							ui.label(RichText::new(status).size(12.0).color(
								if pending.delivery == Delivery::Rejected {
									colors.danger
								} else {
									colors.muted
								},
							));
						});
					}
					if !pending.content.is_empty() {
						ui.scope(|ui| {
							ui.set_opacity(0.55);
							// Pending gray (or failure red) so confirmation visibly replaces the row.
							ui.visuals_mut().override_text_color =
								Some(if pending.delivery == Delivery::Rejected {
									colors.danger
								} else {
									colors.muted
								});
							// Source equality in FormatCache also handles a nonce hash collision.
							let formatted = formats.get(
								model::Id(egui::Id::unique(&pending.nonce).value()),
								&pending.content,
							);
							let id = ui.scope_id().with("spoilers");
							let mut revealed =
								ui.data_mut(|data| data.get_temp::<u32>(id).unwrap_or(0));
							let mut surface = crate::select::Surface::new(ui, "pending-body");
							let jumbo = formatted.jumbo();
							text_line = ui
								.scope(|ui| {
									if jumbo {
										crate::design::jumbo_emoji(ui);
									}
									let source = crate::mentions::MentionSource {
										state,
										channel: pending.channel,
									};
									formatted.show_references(
										ui,
										opening,
										&crate::mentions::known_users(state, pending.channel),
										Some(&source),
										profile,
										(
											&state.channels,
											channel,
											&state.guilds,
											crate::mentions::known_roles(state, pending.channel),
										),
										(avatars, state.demo, &mut revealed),
										&mut surface,
									);
								})
								.response
								.rect;
							surface.finish(ui);
							if revealed != 0 {
								ui.data_mut(|data| data.insert_temp(id, revealed));
							}
						})
						.response
						.on_hover_text(status);
					}
					if let Some(sticker) = &pending.sticker {
						ui.scope(|ui| {
							ui.set_opacity(if sending { 0.55 } else { 1.0 });
							let edge = ui.available_width().min(160.0);
							avatars.sticker_image(ui, sticker, egui::Vec2::splat(edge), state.demo);
						});
					}
					if !pending.attachments.is_empty() {
						ui.scope(|ui| {
							ui.set_max_width(ui.available_width().min(MAX_WIDTH));
							// Discord fades the optimistic row until the server echoes it.
							ui.set_opacity(if sending { 0.6 } else { 1.0 });
							files(ui, pending, upload);
						});
					}
					if sending && !artwork {
						if !pending.attachments.is_empty() {
							ui.add_space(2.0);
							ui.scope(|ui| {
								ui.set_max_width(ui.available_width().min(MAX_WIDTH));
								upload_strip(ui, pending, upload, cancel);
							});
						}
					} else if pending.delivery != Delivery::Confirmed {
						if pending.delivery == Delivery::Ambiguous {
							ui.label(
								RichText::new("Check the conversation before sending again.")
									.small()
									.color(colors.muted),
							);
						}
						if ui
							.button(if pending.sticker.is_some() {
								"Dismiss"
							} else {
								"Restore to composer"
							})
							.clicked()
						{
							*restore = Some(pending.nonce.clone());
						}
					}
					crate::timeline::fill_header_line(ui, compact, text_line);
				});
			});
		});
}

/// Image thumbnails in the confirmed-message grid, then one card per other file.
fn files(ui: &mut egui::Ui, pending: &Pending, upload: Option<&Upload>) {
	let colors = design::palette(ui);
	let file = |index: usize| upload.and_then(|upload| upload.files.get(index));
	let images: Vec<(&str, &egui::TextureHandle)> = pending
		.attachments
		.iter()
		.enumerate()
		.filter_map(|(index, filename)| {
			file(index)
				.and_then(|file| file.preview.as_ref())
				.map(|preview| (filename.as_str(), preview))
		})
		.collect();
	if !images.is_empty() {
		let (columns, size) = attachments::image_layout(images.len(), ui.available_width());
		ui.spacing_mut().item_spacing = egui::vec2(6.0, 6.0);
		for row in images.chunks(columns) {
			ui.horizontal_top(|ui| {
				for (filename, texture) in row {
					let source = texture.size_vec2();
					let scale = (size.x / source.x).min(size.y / source.y);
					let artwork = attachments::artwork_edge(filename);
					let fitted = if let Some(edge) = artwork {
						egui::Vec2::splat(edge.min(size.x).min(size.y))
					} else if images.len() > 1 {
						// Grid tiles share one height so rows stay aligned, like Discord.
						egui::vec2(size.x, size.y)
					} else {
						source * scale.clamp(f32::EPSILON, 1.0)
					};
					let (rect, _) = ui.allocate_exact_size(fitted, egui::Sense::hover());
					if artwork.is_none() {
						ui.painter().rect_filled(rect, 8, colors.raised);
					}
					let shown = if images.len() > 1 {
						egui::Rect::from_center_size(rect.center(), source * scale).intersect(rect)
					} else {
						rect
					};
					ui.put(
						shown,
						egui::Image::from_texture(*texture)
							.fit_to_exact_size(shown.size())
							.corner_radius(8),
					);
				}
			});
		}
	}
	for (index, filename) in pending.attachments.iter().enumerate() {
		if file(index).is_some_and(|file| file.preview.is_some()) {
			continue;
		}
		let kind = attachments::file_kind(filename, None);
		egui::Frame::new()
			.fill(colors.raised)
			.stroke(egui::Stroke::new(1.0, colors.border))
			.corner_radius(8)
			.inner_margin(egui::Margin::symmetric(12, 10))
			.show(ui, |ui| {
				ui.set_width(ui.available_width());
				ui.horizontal(|ui| {
					ui.spacing_mut().item_spacing.x = 10.0;
					icons::inline(ui, kind.icon(), 32.0, kind.tint(&colors));
					ui.vertical(|ui| {
						ui.spacing_mut().item_spacing.y = 0.0;
						ui.add(
							egui::Label::new(
								design::medium(ui, filename, 14.0).color(colors.text_strong),
							)
							.truncate()
							.selectable(false),
						)
						.on_hover_text(filename);
						if let Some(file) = file(index) {
							ui.add(
								egui::Label::new(
									RichText::new(attachments::format_size(file.bytes))
										.size(12.0)
										.color(colors.muted),
								)
								.selectable(false),
							);
						}
					});
				});
			});
	}
}

/// One compact progress card for the whole batch, with Discord's dismiss cross.
fn upload_strip(ui: &mut egui::Ui, pending: &Pending, upload: Option<&Upload>, cancel: &mut bool) {
	let colors = design::palette(ui);
	let count = pending.attachments.len();
	let title = if count == 1 {
		format!("Uploading {}", pending.attachments[0])
	} else {
		format!("Uploading {count} files")
	};
	let (fraction, detail) = match upload.and_then(|u| u.progress) {
		Some((sent, total)) if total > 0 && sent < total => (
			(sent as f32 / total as f32).clamp(0.0, 1.0),
			format!(
				"{} of {} · {}%",
				attachments::format_size(sent),
				attachments::format_size(total),
				sent.saturating_mul(100) / total
			),
		),
		Some((_, total)) if total > 0 => (1.0, "Sending message…".into()),
		_ => (
			0.0,
			match upload {
				Some(upload) => format!(
					"Preparing upload · {}",
					attachments::format_size(upload.bytes())
				),
				None => "Preparing upload…".into(),
			},
		),
	};
	egui::Frame::new()
		.fill(colors.raised)
		.stroke(egui::Stroke::new(1.0, colors.border))
		.corner_radius(8)
		.inner_margin(egui::Margin::symmetric(12, 10))
		.show(ui, |ui| {
			ui.set_width(ui.available_width());
			ui.spacing_mut().item_spacing.y = 4.0;
			ui.horizontal(|ui| {
				ui.spacing_mut().item_spacing.x = 8.0;
				ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
					if upload.is_some()
						&& icons::button(ui, icons::Icon::Close, 24.0, "Cancel upload")
							.on_hover_text(
								"The message may already have reached Discord. Check the conversation before sending again.",
							)
							.clicked()
					{
						*cancel = true;
					}
					ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
						ui.vertical(|ui| {
							ui.spacing_mut().item_spacing.y = 0.0;
							ui.add(
								egui::Label::new(
									design::medium(ui, title, 14.0).color(colors.text_strong),
								)
								.truncate()
								.selectable(false),
							);
							ui.add(
								egui::Label::new(
									RichText::new(detail).size(12.0).color(colors.muted),
								)
								.truncate()
								.selectable(false),
							);
						});
					});
				});
			});
			let (bar, _) = ui.allocate_exact_size(
				egui::vec2(ui.available_width(), 6.0),
				egui::Sense::hover(),
			);
			ui.painter().rect_filled(bar, 3, colors.hover);
			if fraction > 0.0 {
				let fill = egui::Rect::from_min_size(
					bar.min,
					egui::vec2((bar.width() * fraction).max(6.0), bar.height()),
				);
				ui.painter().rect_filled(fill, 3, colors.accent);
			}
		});
}

#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn pending_content_progress_and_failure_remain_visible() {
		fn text(shape: &egui::Shape, result: &mut String) {
			match shape {
				egui::Shape::Text(t) => result.push_str(&t.galley.job.text),
				egui::Shape::Vec(shapes) => shapes.iter().for_each(|s| text(s, result)),
				_ => {}
			}
		}
		let ctx = egui::Context::default();
		let state = State {
			demo: true,
			..Default::default()
		};
		let mut pending = Pending {
			sticker: None,
			channel: model::Id(1),
			content: format!("{} FULL END", "Full message text ".repeat(10)),
			attachments: vec!["notes.txt".into(), "photo.png".into()],
			nonce: "local".into(),
			delivery: Delivery::Sending,
			confirmed: None,
		};
		let mut upload = Upload {
			nonce: pending.nonce.clone(),
			files: vec![
				UploadFile {
					bytes: 40,
					preview: None,
				},
				UploadFile {
					bytes: 60,
					preview: Some(ctx.load_texture(
						"synthetic-photo",
						egui::ColorImage::filled([4, 3], egui::Color32::WHITE),
						egui::TextureOptions::LINEAR,
					)),
				},
			],
			progress: Some((25, 100)),
		};
		assert_eq!(upload.bytes(), 100);
		for (delivery, progress, expected) in [
			(Delivery::Sending, Some((25, 100)), "Uploading 2 files"),
			(
				Delivery::Sending,
				Some((25, 100)),
				"25 bytes of 100 bytes · 25%",
			),
			(Delivery::Sending, Some((100, 100)), "Sending message…"),
			(Delivery::Sending, None, "Preparing upload · 100 bytes"),
			(
				Delivery::Sending,
				Some((0, 0)),
				"Preparing upload · 100 bytes",
			),
			(Delivery::Rejected, None, "Not sent"),
			(Delivery::Ambiguous, None, "Delivery unknown"),
		] {
			pending.delivery = delivery;
			upload.progress = progress;
			let mut painted = String::new();
			let mut profile = crate::profiles::ProfileSession::default();
			for _ in 0..2 {
				let output = ctx.run_ui(
					egui::RawInput {
						screen_rect: Some(egui::Rect::from_min_size(
							egui::Pos2::ZERO,
							egui::vec2(260.0, 900.0),
						)),
						..Default::default()
					},
					|ui| {
						show(
							ui,
							&pending,
							true,
							&state,
							(
								&mut crate::avatars::Avatars::default(),
								&mut None,
								&mut profile,
								&mut None,
								&mut crate::markdown::FormatCache::default(),
							),
							Some(&upload),
							(&mut None, &mut false),
						);
					},
				);
				painted.clear();
				for shape in &output.shapes {
					text(&shape.shape, &mut painted);
				}
				output.drop_without_applying_deltas();
			}
			assert!(painted.contains("FULL END"), "{painted}");
			assert!(painted.contains(expected), "{painted}");
			// Every file stays visible: the text file as a card, the image as a thumbnail only.
			assert!(painted.contains("notes.txt"), "{painted}");
			assert!(!painted.contains("photo.png"), "{painted}");
			assert_eq!(
				painted.contains("Restore to composer"),
				delivery != Delivery::Sending
			);
		}
	}
}
