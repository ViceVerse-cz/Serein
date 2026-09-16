//! Fixed-height native invite cards; only visible cards request bounded metadata.
use crate::avatars::Avatars;
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
	ui.label(
		egui::RichText::new(format!("{value} {label}"))
			.size(13.0)
			.color(colors.muted),
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
							let eyebrow = if member {
								"You're a member of"
							} else if failed {
								"Invite unavailable"
							} else {
								"You've been invited to join a server"
							};
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
									images.show_embed(&mut icon_ui, icon, icon_rect.size(), demo);
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

								// Action first so the text column gets whatever width remains.
								let label = if member {
									"Go To Server"
								} else if verification {
									"Verify"
								} else if pending {
									"Joining…"
								} else if accepted {
									"Accepted"
								} else {
									"Join"
								};
								ui.with_layout(
									egui::Layout::right_to_left(egui::Align::Center),
									|ui| {
										let enabled =
											member || verification || state.can_join_invite(&code);
										ui.add_enabled_ui(enabled, |ui| {
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
												crate::design::semibold(ui, label, 14.0)
													.color(text),
											)
											.fill(fill)
											.stroke(egui::Stroke::NONE)
											.corner_radius(6)
											.min_size(egui::vec2(72.0, 36.0));
											if ui.add(button).clicked() {
												*action = Some(
													if let Some(preview) =
														preview.filter(|_| member)
													{
														Action::OpenGuild(preview.guild)
													} else {
														Action::Join(code.clone())
													},
												);
											}
										});
										ui.with_layout(
											egui::Layout::top_down(egui::Align::Min),
											|ui| {
												ui.spacing_mut().item_spacing.y = 4.0;
												ui.add_space(4.0);
												let title = embed
													.and_then(|e| e.title.as_deref())
													.unwrap_or(if failed {
														"Invite expired or invalid"
													} else if demo {
														"Server preview"
													} else {
														"Loading…"
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
														.filter(|_| {
															join_error.is_none() && !verification
														}) {
														Some((online, members)) => {
															dot_stat(
																ui,
																colors.positive,
																online,
																"Online",
															);
															ui.add_space(6.0);
															dot_stat(
																ui,
																colors.muted,
																members,
																"Members",
															);
														}
														None => {
															let text = if verification {
																"Verification required"
															} else if let Some(f) = join_error {
																f.label()
															} else if let Some(d) =
																embed.and_then(|e| {
																	e.description.as_deref()
																}) {
																d
															} else if demo {
																"Preview unavailable offline"
															} else if failed {
																"This invite may have expired"
															} else {
																"Fetching server details…"
															};
															ui.add(
																egui::Label::new(
																	egui::RichText::new(text)
																		.size(13.0)
																		.color(if verification {
																			colors.accent
																		} else if join_error
																			.is_some()
																		{
																			colors.danger
																		} else {
																			colors.muted
																		}),
																)
																.truncate()
																.selectable(false),
															)
															.on_hover_text(text);
														}
													}
												});
											},
										);
									},
								);
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
