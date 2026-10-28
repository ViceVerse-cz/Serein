//! Fixed-height native invite cards; only visible cards request bounded metadata.
use crate::avatars::{Avatars, Surface};
use client_core::invites::valid_code;
use model::Message;

pub(crate) enum Action {
	Join(String),
	OpenGuild(model::Id),
}
fn code(raw: &str) -> Option<String> {
	let normalized;
	let raw = if raw.starts_with("discord.gg/") || raw.starts_with("discord.com/invite/") {
		normalized = format!("https://{raw}");
		&normalized
	} else {
		raw
	};
	let url = url::Url::parse(raw).ok()?;
	if !matches!(url.scheme(), "https" | "http")
		|| !url.username().is_empty()
		|| url.password().is_some()
		|| url.port().is_some()
	{
		return None;
	}
	let path = url.path().trim_end_matches('/');
	let code = match url.host_str()? {
		"discord.gg" | "www.discord.gg" => path.strip_prefix('/')?,
		"discord.com" | "www.discord.com" | "discordapp.com" | "www.discordapp.com" => {
			path.strip_prefix("/invite/")?
		}
		_ => return None,
	};
	valid_code(code).then(|| code.to_owned())
}
pub(super) fn input_code(raw: &str) -> Option<String> {
	if raw.len() > 2048 {
		return None;
	}
	let raw = raw.trim();
	if valid_code(raw) {
		Some(raw.to_owned())
	} else {
		code(raw)
	}
}
fn codes(message: &Message) -> Vec<String> {
	if message.embeds_suppressed || message.content.contains("||") {
		return Vec::new();
	}
	let mut found = Vec::new();
	let mut in_code = false;
	for event in pulldown_cmark::Parser::new(&message.content) {
		use pulldown_cmark::{Event, Tag, TagEnd};
		match event {
			Event::Start(Tag::CodeBlock(_)) => in_code = true,
			Event::End(TagEnd::CodeBlock) => in_code = false,
			Event::Start(Tag::Link { dest_url, .. }) if !in_code => {
				if let Some(code) = code(&dest_url)
					&& !found.contains(&code)
				{
					found.push(code);
				}
			}
			Event::Text(text) if !in_code => {
				for token in text.split_whitespace() {
					let token = token.trim_matches(|c: char| {
						matches!(
							c,
							'<' | '>' | '(' | ')' | '[' | ']' | ',' | '.' | '!' | '?' | ';' | '"'
						)
					});
					if let Some(code) = code(token)
						&& !found.contains(&code)
					{
						found.push(code);
					}
					if found.len() == 3 {
						break;
					}
				}
			}
			_ => {}
		}
		if found.len() == 3 {
			break;
		}
	}
	found
}
const CARD_HEIGHT: f32 = 108.0;
pub fn estimated_height(message: &Message) -> f32 {
	codes(message).len() as f32 * (CARD_HEIGHT + 4.0)
}
/// Splits the protocol's "● N online · M members" description into its two counts.
pub(super) fn counts(description: &str) -> Option<(&str, &str)> {
	let (online, members) = description.split_once(" · ")?;
	let online = online
		.trim_start_matches('●')
		.trim()
		.strip_suffix(" online")?;
	let members = members.trim().strip_suffix(" members")?;
	Some((online, members))
}
pub(super) fn dot_stat(ui: &mut egui::Ui, color: egui::Color32, value: &str, label: &str) {
	let colors = crate::design::palette(ui);
	let (rect, _) = ui.allocate_exact_size(egui::vec2(8.0, 8.0), egui::Sense::hover());
	ui.painter().circle_filled(rect.center(), 4.0, color);
	ui.add_space(-2.0);
	ui.add(
		egui::Label::new(
			egui::RichText::new(format!("{value} {}", crate::i18n::translate_if_key(label)))
				.size(13.0)
				.color(colors.muted),
		)
		.truncate()
		.selectable(false),
	);
}
pub fn show(
	ui: &mut egui::Ui,
	message: &Message,
	state: &client_core::State,
	images: &mut Avatars,
	requests: &mut Vec<String>,
	action: &mut Option<Action>,
) {
	let demo = state.demo;
	for code in codes(message) {
		ui.push_id(("invite", &code), |ui| {
			let colors = crate::design::palette(ui);
			let width = ui.available_width().min(432.0);
			let entry = state
				.invites
				.get(&code)
				.filter(|(at, _)| at.elapsed().as_secs() < 300);
			let preview = entry
				.and_then(|(_, value)| value.as_ref())
				.and_then(|r| r.as_ref().ok());
			let member = preview.is_some_and(|p| state.guild(p.guild).is_some());
			let embed = preview.map(|p| &p.embed);
			let failed = entry.is_some_and(|(_, v)| matches!(v, Some(Err(_))));
			if !demo && entry.is_none() && requests.len() < 8 && !requests.contains(&code) {
				requests.push(code.clone());
			}
			let current = state.invite_join.code == code;
			let verification = current && state.invite_challenge().is_some();
			let pending = current && state.invite_join.pending;
			let accepted = current && matches!(state.invite_join.result, Some(Ok(_)));
			let join_error = current
				.then_some(state.invite_join.result.as_ref())
				.flatten()
				.and_then(|r| r.as_ref().err());

			ui.allocate_ui_with_layout(
				egui::vec2(width, CARD_HEIGHT),
				egui::Layout::top_down(egui::Align::Min),
				|ui| {
					egui::Frame::new()
						.fill(colors.raised)
						.corner_radius(8)
						.stroke(egui::Stroke::new(1.0, colors.border))
						.inner_margin(16)
						.show(ui, |ui| {
							ui.set_width(width - 32.0);
							ui.set_height(CARD_HEIGHT - 32.0);
							ui.spacing_mut().item_spacing.y = 0.0;
							let eyebrow = crate::i18n::translate_if_key(if member {
								"invites-show-you-re-a-member-of"
							} else if failed {
								"invites-show-invite-unavailable"
							} else {
								"invites-show-you-ve-been-invited-to-join-a-server"
							});
							ui.label(crate::design::eyebrow(ui, eyebrow, colors.muted));
							ui.add_space(12.0);
							ui.horizontal(|ui| {
								ui.spacing_mut().item_spacing.x = 16.0;
								let (icon_rect, _) = ui.allocate_exact_size(
									egui::vec2(50.0, 50.0),
									egui::Sense::hover(),
								);
								if let Some(icon) = embed.and_then(|e| e.thumbnail.as_ref()) {
									let mut icon_ui =
										ui.new_child(egui::UiBuilder::new().max_rect(icon_rect));
									images.show_media(
										&mut icon_ui,
										icon,
										icon_rect.size(),
										demo,
										Surface::Inline,
									);
								} else {
									ui.painter().rect_filled(icon_rect, 16, colors.sidebar);
									let initial = embed
										.and_then(|e| e.title.as_deref())
										.and_then(|t| t.chars().next())
										.map(|c| c.to_uppercase().to_string());
									ui.painter().text(
										icon_rect.center(),
										egui::Align2::CENTER_CENTER,
										initial.as_deref().unwrap_or("?"),
										egui::FontId::proportional(20.0),
										colors.text_strong,
									);
								}

								let label = crate::i18n::translate_if_key(if member {
									"invites-show-go-to-server"
								} else if verification {
									"invites-show-verify"
								} else if pending {
									"invites-show-joining"
								} else if accepted {
									"invites-show-accepted"
								} else {
									"invites-show-join"
								});
								let enabled =
									member || verification || state.can_join_invite(&code);
								let fill = if verification {
									colors.accent
								} else if enabled {
									colors.positive
								} else {
									colors.selected
								};
								let text = if enabled {
									egui::Color32::WHITE
								} else {
									colors.muted
								};
								let button = egui::Button::new(
									crate::design::semibold(ui, &label, 14.0).color(text),
								)
								.fill(fill)
								.stroke(egui::Stroke::NONE)
								.corner_radius(6)
								.min_size(egui::vec2(72.0, 36.0));

								let font_id = egui::FontId::new(
									14.0,
									crate::design::semibold_family(ui.ctx()),
								);
								let galley =
									ui.painter().layout_no_wrap(label.clone(), font_id, text);
								let button_padding = ui.spacing().button_padding.x * 2.0;
								let button_width = (galley.size().x + button_padding).max(72.0);
								let text_width =
									(ui.available_width() - button_width - 12.0).max(0.0);

								ui.allocate_ui_with_layout(
									egui::vec2(text_width, 50.0),
									egui::Layout::top_down(egui::Align::Min),
									|ui| {
										ui.spacing_mut().item_spacing.y = 4.0;
										ui.add_space(4.0);
										let title = embed
											.and_then(|e| e.title.clone())
											.unwrap_or_else(|| {
												crate::i18n::translate_if_key(if failed {
													"invites-show-invite-expired-or-invalid"
												} else if demo {
													"invites-show-server-preview"
												} else {
													"invites-show-loading"
												})
											});
										ui.add(
											egui::Label::new(
												crate::design::semibold(ui, title, 16.0)
													.color(colors.text_strong),
											)
											.truncate()
											.selectable(false),
										);
										ui.horizontal(|ui| {
											ui.spacing_mut().item_spacing.x = 6.0;
											match embed
												.and_then(|e| e.description.as_deref())
												.and_then(counts)
												.filter(|_| join_error.is_none() && !verification)
											{
												Some((online, members)) => {
													dot_stat(
														ui,
														colors.positive,
														online,
														"server-settings-preview-online",
													);
													ui.add_space(6.0);
													dot_stat(
														ui,
														colors.muted,
														members,
														"server-settings-preview-members",
													);
												}
												None => {
													let text = if verification {
														crate::i18n::translate(
															"invites-show-verification-required",
														)
													} else if let Some(f) = join_error {
														f.label().to_owned()
													} else if let Some(d) =
														embed.and_then(|e| e.description.as_deref())
													{
														d.to_owned()
													} else if demo {
														crate::i18n::translate(
															"invites-show-preview-unavailable-offline",
														)
													} else if failed {
														crate::i18n::translate(
															"invites-show-this-invite-may-have-expired",
														)
													} else {
														crate::i18n::translate(
															"invites-show-fetching-server-details",
														)
													};
													ui.add(
														egui::Label::new(
															egui::RichText::new(&text)
																.size(13.0)
																.color(if verification {
																	colors.accent
																} else if join_error.is_some() {
																	colors.danger
																} else {
																	colors.muted
																}),
														)
														.truncate()
														.selectable(false),
													)
													.on_hover_text(&text);
												}
											}
										});
									},
								);
								ui.add_space(12.0);
								ui.add_enabled_ui(enabled, |ui| {
									if ui.add(button).clicked() {
										*action = Some(
											if let Some(preview) = preview.filter(|_| member) {
												Action::OpenGuild(preview.guild)
											} else {
												Action::Join(code.clone())
											},
										);
									}
								});
							});
						});
				},
			);
			ui.add_space(4.0);
		});
	}
}
#[cfg(test)]
mod tests {
	#[test]
	fn dot_stat_uses_localized_labels() {
		let prev_lang = crate::i18n::current();
		crate::i18n::set_current(crate::i18n::Language::German);
		let online_label = crate::i18n::translate_if_key("server-settings-preview-online");
		let members_label = crate::i18n::translate_if_key("server-settings-preview-members");
		assert_eq!(online_label, "Online");
		assert_eq!(members_label, "Mitglieder");

		let ctx = egui::Context::default();
		let raw_input = egui::RawInput {
			screen_rect: Some(egui::Rect::from_min_size(
				egui::Pos2::ZERO,
				egui::vec2(600.0, 400.0),
			)),
			..Default::default()
		};
		fn find_text(shapes: &[egui::epaint::ClippedShape], needle: &str) -> bool {
			fn check_shape(s: &egui::Shape, needle: &str) -> bool {
				match s {
					egui::Shape::Text(t) => t.galley.job.text.contains(needle),
					egui::Shape::Vec(v) => v.iter().any(|shape| check_shape(shape, needle)),
					_ => false,
				}
			}
			shapes.iter().any(|s| check_shape(&s.shape, needle))
		}

		let output = ctx.run_ui(raw_input.clone(), |ui| {
			super::dot_stat(
				ui,
				egui::Color32::WHITE,
				"340",
				"server-settings-preview-members",
			);
			super::dot_stat(
				ui,
				egui::Color32::WHITE,
				"12",
				"server-settings-preview-online",
			);
		});
		let found_members = find_text(&output.shapes, "Mitglieder");
		let found_online = find_text(&output.shapes, "Online");
		output.drop_without_applying_deltas();
		assert!(found_members);
		assert!(found_online);

		let mut state = test_support::demo_state();
		let code = "synthetic-invite".to_string();
		state.invites.insert(
			code.clone(),
			(
				std::time::Instant::now(),
				Some(Ok(model::InvitePreview {
					guild: model::Id(999),
					embed: model::Embed {
						title: Some("Rust Server".into()),
						description: Some("● 12 online · 340 members".into()),
						..Default::default()
					},
				})),
			),
		);
		let mut message = test_support::message(1, model::Id(2));
		message.content = format!("https://discord.gg/{code}");
		let mut avatars = crate::avatars::Avatars::default();
		let mut requests = Vec::new();
		let mut action = None;
		let card_output = ctx.run_ui(raw_input, |ui| {
			super::show(
				ui,
				&message,
				&state,
				&mut avatars,
				&mut requests,
				&mut action,
			);
		});
		let card_members = find_text(&card_output.shapes, "Mitglieder");
		let card_online = find_text(&card_output.shapes, "Online");
		card_output.drop_without_applying_deltas();
		assert!(card_members);
		assert!(card_online);

		crate::i18n::set_current(prev_lang);
	}

	#[test]
	fn standalone_invites_accept_codes_without_turning_plain_messages_into_cards() {
		assert_eq!(
			super::input_code("  synthetic-123  ").as_deref(),
			Some("synthetic-123")
		);
		assert_eq!(
			super::input_code("https://discord.gg/synthetic").as_deref(),
			Some("synthetic")
		);
		assert!(super::code("synthetic").is_none());
		for raw in [
			"",
			"../bad",
			"https://discord.gg.evil.test/code",
			"https://user@discord.gg/code",
			"https://discord.gg/a/b",
			"two codes",
		] {
			assert!(super::input_code(raw).is_none(), "{raw}");
		}
		assert!(super::input_code(&"a".repeat(2049)).is_none());
	}
	#[test]
	fn invite_urls_are_origin_and_path_checked() {
		assert_eq!(
			super::code("https://discord.gg/abc-123").as_deref(),
			Some("abc-123")
		);
		assert_eq!(
			super::code("https://discord.com/invite/abc").as_deref(),
			Some("abc")
		);
		for bad in [
			"https://discord.gg.evil.test/abc",
			"https://discord.gg@evil.test/abc",
			"https://evil@discord.gg/abc",
			"https://discord.gg/a/b",
			"https://discord.gg/%2e%2e",
			"https://discord.gg/",
		] {
			assert!(super::code(bad).is_none());
		}
	}
}
