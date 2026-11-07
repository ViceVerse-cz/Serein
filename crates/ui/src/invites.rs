//! Discord-style server and voice channel invite cards; only visible cards request bounded
//! metadata.
use crate::avatars::{Avatars, Surface};
use client_core::invites::valid_code;
use model::Message;

pub(crate) enum Action {
	Join(String),
	OpenGuild(model::Id),
	OpenChannel(model::Id),
	/// A member's voice channel invite: join the call there.
	JoinVoice(model::Id),
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
// Proportions measured from the official client's card.
const CARD_WIDTH: f32 = 320.0;
const CARD_RADIUS: u8 = 10;
const BANNER: f32 = 76.0;
const ICON: f32 = 64.0;
/// How far the icon rises into the banner.
const ICON_RISE: f32 = 37.0;
/// Inset of the card's text and action.
const PAD: f32 = 14.0;
/// A typical server card with a one-line description; rows are remeasured once drawn.
const ESTIMATED_CARD: f32 = 250.0;
pub fn estimated_height(message: &Message) -> f32 {
	codes(message).len() as f32 * (ESTIMATED_CARD + 4.0)
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
/// The card's single action.
struct Button {
	label: String,
	enabled: bool,
	fill: egui::Color32,
	icon: Option<crate::icons::Icon>,
}

/// What the card says under its title while it cannot show the server's numbers.
struct Status {
	text: String,
	color: egui::Color32,
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
			let width = ui.available_width().min(CARD_WIDTH);
			let entry = state
				.invites
				.get(&code)
				.filter(|(at, _)| at.elapsed().as_secs() < 300);
			let preview = entry
				.and_then(|(_, value)| value.as_ref())
				.and_then(|r| r.as_ref().ok());
			let member = preview.is_some_and(|p| {
				p.guild.is_some_and(|g| state.guild(g).is_some())
					|| p.channel.is_some_and(|c| state.channel(c).is_some())
			});
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
			let voice = preview.and_then(|p| p.voice_channel.as_deref());
			// A member's voice invite joins the call; everything else joins or opens the server.
			let voice_channel = preview
				.and_then(|p| p.channel)
				.filter(|&channel| voice.is_some() && member && state.channel(channel).is_some());
			let enabled = member || verification || state.can_join_invite(&code);
			let button = Button {
				label: crate::i18n::translate_if_key(if voice_channel.is_some() {
					"invites-show-join-voice"
				} else if member {
					"invites-show-go-to-server"
				} else if verification {
					"invites-show-verify"
				} else if pending {
					"invites-show-joining"
				} else if accepted {
					"invites-show-accepted"
				} else {
					"invites-show-join"
				}),
				enabled,
				fill: colors.accent,
				icon: voice_channel.map(|_| crate::icons::Icon::Speaker),
			};
			let status = if verification {
				Some(Status {
					text: crate::i18n::translate("invites-show-verification-required"),
					color: colors.accent,
				})
			} else {
				join_error.map(|error| Status {
					text: error.label().to_owned(),
					color: colors.danger,
				})
			};
			let clicked = match voice {
				Some(name) => voice_card(
					ui,
					width,
					name,
					preview.and_then(|p| p.channel).filter(|_| member),
					state,
					images,
					status,
					&button,
				),
				None => {
					let embed = preview.map(|p| &p.embed);
					let title = embed.and_then(|e| e.title.clone()).unwrap_or_else(|| {
						crate::i18n::translate_if_key(if failed {
							"invites-show-invite-expired-or-invalid"
						} else if demo {
							"invites-show-server-preview"
						} else {
							"invites-show-loading"
						})
					});
					let status = status.or_else(|| {
						let counted = embed
							.and_then(|e| e.description.as_deref())
							.and_then(counts)
							.is_some();
						(!counted).then(|| Status {
							text: if let Some(d) = embed.and_then(|e| e.description.as_deref()) {
								d.to_owned()
							} else if demo {
								crate::i18n::translate("invites-show-preview-unavailable-offline")
							} else if failed {
								crate::i18n::translate("invites-show-this-invite-may-have-expired")
							} else {
								crate::i18n::translate("invites-show-fetching-server-details")
							},
							color: colors.muted,
						})
					});
					server_card(ui, width, &title, preview, status, images, demo, &button)
				}
			};
			if clicked {
				*action = Some(if let Some(channel) = voice_channel {
					Action::JoinVoice(channel)
				} else if let Some(preview) = preview.filter(|_| member) {
					if let Some(guild) = preview.guild.filter(|&g| state.guild(g).is_some()) {
						Action::OpenGuild(guild)
					} else if let Some(channel) =
						preview.channel.filter(|&c| state.channel(c).is_some())
					{
						Action::OpenChannel(channel)
					} else {
						Action::Join(code.clone())
					}
				} else {
					Action::Join(code.clone())
				});
			}
			ui.add_space(4.0);
		});
	}
}

/// Discord's server invite: banner, an icon overlapping it, name, counts, age, description
/// and one full-width action.
#[allow(clippy::too_many_arguments)]
fn server_card(
	ui: &mut egui::Ui,
	width: f32,
	title: &str,
	preview: Option<&model::InvitePreview>,
	status: Option<Status>,
	images: &mut Avatars,
	demo: bool,
	button: &Button,
) -> bool {
	let colors = crate::design::palette(ui);
	let detail = colors.text.lerp_to_gamma(colors.muted, 0.4);
	let embed = preview.map(|p| &p.embed);
	let mut clicked = false;
	egui::Frame::new()
		.fill(colors.raised)
		.corner_radius(CARD_RADIUS)
		.stroke(egui::Stroke::new(1.0, colors.border))
		.show(ui, |ui| {
			ui.set_width(width);
			ui.spacing_mut().item_spacing.y = 0.0;
			let (banner, _) =
				ui.allocate_exact_size(egui::vec2(width, BANNER), egui::Sense::hover());
			let top = egui::CornerRadius {
				nw: CARD_RADIUS,
				ne: CARD_RADIUS,
				sw: 0,
				se: 0,
			};
			ui.painter()
				.rect_filled(banner, top, colors.accent.gamma_multiply(0.35));
			if let Some(image) = embed.and_then(|e| e.image.as_ref()) {
				// Paint past the banner's bottom and clip it, so only the top corners round.
				let mut child = ui.new_child(
					egui::UiBuilder::new().max_rect(banner.with_max_y(banner.bottom() + 24.0)),
				);
				child.set_clip_rect(banner.intersect(ui.clip_rect()));
				images.show_media(
					&mut child,
					image,
					egui::vec2(width, BANNER + 24.0),
					demo,
					Surface::Banner,
				);
			}
			let icon = egui::Rect::from_min_size(
				egui::pos2(banner.left() + PAD, banner.bottom() - ICON_RISE),
				egui::Vec2::splat(ICON),
			);
			// The card's own fill rings the icon where it overlaps the banner.
			ui.painter()
				.rect_filled(icon.expand(4.0), 22, colors.raised);
			match (preview, preview.and_then(|p| p.guild)) {
				(Some(preview), Some(guild)) if preview.guild_icon.is_some() => {
					let guild = model::Guild {
						default_message_notifications: None,
						stickers: None,
						emojis: None,
						id: guild,
						name: title.to_owned(),
						icon: preview.guild_icon.clone(),
					};
					images.paint_guild(ui, &guild, icon, demo, 18);
				}
				_ => {
					ui.painter().rect_filled(icon, 18, colors.sidebar);
					let initial = title
						.chars()
						.next()
						.map(|c| c.to_uppercase().to_string())
						.unwrap_or_else(|| "?".into());
					ui.painter().text(
						icon.center(),
						egui::Align2::CENTER_CENTER,
						initial,
						egui::FontId::proportional(24.0),
						colors.text_strong,
					);
				}
			}
			ui.add_space(ICON - ICON_RISE + 10.0);
			egui::Frame::new()
				.inner_margin(egui::Margin {
					left: PAD as i8,
					right: PAD as i8,
					top: 0,
					bottom: PAD as i8,
				})
				.show(ui, |ui| {
					ui.set_width(width - 2.0 * PAD);
					ui.spacing_mut().item_spacing.y = 2.0;
					ui.add(
						egui::Label::new(
							crate::design::semibold(ui, title, 16.0).color(colors.text_strong),
						)
						.truncate()
						.selectable(false),
					);
					ui.add_space(2.0);
					match status {
						Some(status) => {
							ui.add(
								egui::Label::new(
									egui::RichText::new(&status.text)
										.size(13.0)
										.color(status.color),
								)
								.truncate()
								.selectable(false),
							)
							.on_hover_text(&status.text);
						}
						None => {
							if let Some((online, members)) = embed
								.and_then(|e| e.description.as_deref())
								.and_then(counts)
							{
								ui.horizontal(|ui| {
									ui.spacing_mut().item_spacing.x = 5.0;
									stat(
										ui,
										colors.positive,
										online,
										"server-settings-preview-online",
										detail,
									);
									ui.add_space(8.0);
									stat(
										ui,
										detail,
										members,
										"server-settings-preview-members",
										detail,
									);
								});
							}
							if let Some(established) =
								preview.and_then(|p| p.guild).and_then(established)
							{
								ui.label(egui::RichText::new(established).size(13.0).color(detail));
							}
							if let Some(about) = preview.and_then(|p| p.about.as_deref()) {
								ui.add_space(8.0);
								let mut job = egui::text::LayoutJob::simple(
									about.to_owned(),
									egui::FontId::proportional(13.0),
									detail,
									width - 2.0 * PAD,
								);
								job.wrap.max_rows = 3;
								job.wrap.overflow_character = Some('…');
								ui.add(egui::Label::new(job).selectable(false));
							}
						}
					}
					ui.add_space(12.0);
					clicked = action_button(ui, button).clicked();
				});
		});
	clicked
}

/// One count beside its coloured dot, at the card's detail size.
fn stat(ui: &mut egui::Ui, dot: egui::Color32, value: &str, label: &str, color: egui::Color32) {
	let (rect, _) = ui.allocate_exact_size(egui::vec2(7.0, 16.0), egui::Sense::hover());
	ui.painter().circle_filled(rect.center(), 3.5, dot);
	ui.add(
		egui::Label::new(
			egui::RichText::new(format!("{value} {}", crate::i18n::translate_if_key(label)))
				.size(13.0)
				.color(color),
		)
		.truncate()
		.selectable(false),
	);
}

/// Discord's voice channel invite: the channel's name over who is in it and a join action.
/// Only members can see the channel's roster.
#[allow(clippy::too_many_arguments)]
fn voice_card(
	ui: &mut egui::Ui,
	width: f32,
	name: &str,
	channel: Option<model::Id>,
	state: &client_core::State,
	images: &mut Avatars,
	status: Option<Status>,
	button: &Button,
) -> bool {
	let colors = crate::design::palette(ui);
	let mut clicked = false;
	egui::Frame::new()
		.fill(colors.raised)
		.corner_radius(CARD_RADIUS)
		.stroke(egui::Stroke::new(1.0, colors.border))
		.show(ui, |ui| {
			ui.set_width(width);
			ui.spacing_mut().item_spacing.y = 0.0;
			let (header, _) = ui.allocate_exact_size(egui::vec2(width, 44.0), egui::Sense::hover());
			let speaker = egui::Rect::from_center_size(
				egui::pos2(header.left() + PAD + 9.0, header.center().y),
				egui::Vec2::splat(18.0),
			);
			crate::icons::paint(
				ui.painter(),
				crate::icons::Icon::Speaker,
				speaker,
				colors.muted,
			);
			let galley = ui.painter().layout(
				name.to_owned(),
				egui::FontId::proportional(15.0),
				colors.muted,
				(header.right() - PAD - speaker.right() - 10.0).max(16.0),
			);
			let galley = if galley.rows.len() > 1 {
				let mut job = egui::text::LayoutJob::simple_singleline(
					name.to_owned(),
					egui::FontId::proportional(15.0),
					colors.muted,
				);
				job.wrap = egui::text::TextWrapping::truncate_at_width(
					(header.right() - PAD - speaker.right() - 10.0).max(16.0),
				);
				ui.painter().layout_job(job)
			} else {
				galley
			};
			ui.painter().galley(
				egui::pos2(
					speaker.right() + 10.0,
					header.center().y - galley.size().y / 2.0,
				),
				galley,
				colors.muted,
			);
			egui::Frame::new()
				.fill(colors.sidebar)
				.corner_radius(egui::CornerRadius {
					nw: 0,
					ne: 0,
					sw: CARD_RADIUS,
					se: CARD_RADIUS,
				})
				.inner_margin(PAD)
				.show(ui, |ui| {
					ui.set_width(width - 2.0 * PAD);
					ui.spacing_mut().item_spacing.y = 4.0;
					let roster: Vec<_> = channel
						.map(|channel| {
							state
								.voice
								.roster
								.iter()
								.filter(|entry| entry.channel == channel)
								.collect()
						})
						.unwrap_or_default();
					if let Some(status) = status {
						ui.add(
							egui::Label::new(
								egui::RichText::new(&status.text)
									.size(13.0)
									.color(status.color),
							)
							.truncate()
							.selectable(false),
						);
					} else if roster.is_empty() {
						ui.label(
							egui::RichText::new(crate::i18n::translate(
								"invites-show-no-one-in-voice",
							))
							.size(13.0)
							.color(colors.muted),
						);
					} else {
						ui.horizontal(|ui| {
							ui.spacing_mut().item_spacing.x = 4.0;
							for entry in roster.iter().take(8) {
								match &entry.member {
									Some(member) => {
										images.show_quiet(ui, &member.user, 24.0, state.demo);
									}
									None => {
										let (rect, _) = ui.allocate_exact_size(
											egui::Vec2::splat(24.0),
											egui::Sense::hover(),
										);
										ui.painter().circle_filled(
											rect.center(),
											12.0,
											colors.hover,
										);
									}
								}
							}
							ui.add_space(4.0);
							ui.label(
								egui::RichText::new(crate::i18n::translate_args(
									"invites-show-in-voice",
									&[("count", &roster.len().to_string())],
								))
								.size(12.0)
								.color(colors.muted),
							);
						});
					}
					ui.add_space(12.0);
					clicked = action_button(ui, button).clicked();
				});
		});
	if channel.is_none() {
		ui.add_space(4.0);
		ui.horizontal(|ui| {
			ui.spacing_mut().item_spacing.x = 4.0;
			crate::icons::inline(ui, crate::icons::Icon::EyeSlash, 14.0, colors.muted);
			ui.label(
				egui::RichText::new(crate::i18n::translate("invites-show-members-see-voice"))
					.size(12.0)
					.color(colors.muted),
			);
		});
	}
	clicked
}

/// "Est. Sep 2026", from the server id's creation time.
fn established(guild: model::Id) -> Option<String> {
	let seconds = ((guild.0 >> 22) + 1_420_070_400_000) / 1000;
	let date = time::OffsetDateTime::from_unix_timestamp(i64::try_from(seconds).ok()?).ok()?;
	let month = date.month().to_string();
	Some(crate::i18n::translate_args(
		"invites-show-established",
		&[(
			"date",
			&format!("{} {}", month.get(..3).unwrap_or(&month), date.year()),
		)],
	))
}

/// Full-width card action: filled, rounded, with an optional leading icon.
fn action_button(ui: &mut egui::Ui, button: &Button) -> egui::Response {
	let colors = crate::design::palette(ui);
	let (rect, response) = ui.allocate_exact_size(
		egui::vec2(ui.available_width(), 34.0),
		if button.enabled {
			egui::Sense::click()
		} else {
			egui::Sense::hover()
		},
	);
	response.widget_info(|| {
		egui::WidgetInfo::labeled(egui::Role::Button, button.enabled, &button.label)
	});
	let fill = if !button.enabled {
		colors.selected
	} else if response.is_pointer_button_down_on() {
		button.fill.lerp_to_gamma(egui::Color32::BLACK, 0.22)
	} else if response.hovered() || response.has_focus() {
		button.fill.lerp_to_gamma(egui::Color32::BLACK, 0.12)
	} else {
		button.fill
	};
	let text = if button.enabled {
		colors.accent_text
	} else {
		colors.muted
	};
	let painter = ui.painter();
	painter.rect_filled(rect, 6, fill);
	painter.rect_stroke(
		rect,
		6,
		egui::Stroke::new(
			1.0,
			egui::Color32::from_white_alpha(if button.enabled { 24 } else { 0 }),
		),
		egui::StrokeKind::Inside,
	);
	if response.has_focus() {
		painter.rect_stroke(
			rect.expand(2.0),
			10,
			egui::Stroke::new(2.0, colors.accent),
			egui::StrokeKind::Outside,
		);
	}
	let galley = painter.layout_no_wrap(
		button.label.clone(),
		egui::FontId::new(14.0, crate::design::medium_family(ui.ctx())),
		text,
	);
	let icon = if button.icon.is_some() {
		18.0 + 6.0
	} else {
		0.0
	};
	let left = rect.center().x - (icon + galley.size().x) / 2.0;
	if let Some(glyph) = button.icon {
		crate::icons::paint(
			painter,
			glyph,
			egui::Rect::from_center_size(
				egui::pos2(left + 9.0, rect.center().y),
				egui::Vec2::splat(18.0),
			),
			text,
		);
	}
	painter.galley(
		egui::pos2(left + icon, rect.center().y - galley.size().y / 2.0),
		galley,
		text,
	);
	if button.enabled && response.hovered() {
		ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
	}
	response
}
#[cfg(test)]
mod tests {
	#[test]
	fn localized_invite_stats_do_not_overlap_action() {
		let prev_lang = crate::i18n::current();
		crate::i18n::set_current(crate::i18n::Language::German);
		let online_label = crate::i18n::translate_if_key("server-settings-preview-online");
		let members_label = crate::i18n::translate_if_key("server-settings-preview-members");

		let ctx = egui::Context::default();
		let raw_input = egui::RawInput {
			screen_rect: Some(egui::Rect::from_min_size(
				egui::Pos2::ZERO,
				egui::vec2(600.0, 400.0),
			)),
			..Default::default()
		};
		let mut state = test_support::demo_state();
		let code = "synthetic-invite".to_string();
		state.invites.insert(
			code.clone(),
			(
				std::time::Instant::now(),
				Some(Ok(model::InvitePreview {
					guild: Some(model::Id(999)),
					about: None,
					guild_icon: None,
					voice_channel: None,
					channel: None,
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
		let mut text_rects = Vec::new();
		let mut rect_shapes = Vec::new();
		let mut worklist: Vec<&egui::Shape> = card_output.shapes.iter().map(|s| &s.shape).collect();
		while let Some(shape) = worklist.pop() {
			match shape {
				egui::Shape::Text(t) => {
					let rect = t.galley.rect.translate(t.pos.to_vec2());
					text_rects.push((t.galley.job.text.clone(), rect));
				}
				egui::Shape::Rect(r) => {
					rect_shapes.push(r.rect);
				}
				egui::Shape::Vec(v) => {
					worklist.extend(v.iter());
				}
				_ => {}
			}
		}

		let members_rect = text_rects
			.iter()
			.find(|(t, _)| t.contains(&members_label))
			.map(|(_, r)| *r)
			.expect("members label rect");
		let online_rect = text_rects
			.iter()
			.find(|(t, _)| t.contains(&online_label))
			.map(|(_, r)| *r)
			.expect("online label rect");
		let join_label = crate::i18n::translate_if_key("invites-show-join");
		let button_text_rect = text_rects
			.iter()
			.find(|(t, _)| t.contains(&join_label))
			.map(|(_, r)| *r)
			.expect("button text rect");
		let button_rect = rect_shapes
			.iter()
			.find(|r| r.contains_rect(button_text_rect))
			.copied()
			.unwrap_or(button_text_rect);
		let card_rect = rect_shapes
			.iter()
			.max_by(|a, b| (a.width() * a.height()).total_cmp(&(b.width() * b.height())))
			.copied()
			.expect("card rect");

		assert!(card_rect.contains_rect(button_rect));
		assert!(!button_rect.intersects(members_rect));
		assert!(!button_rect.intersects(online_rect));
		card_output.drop_without_applying_deltas();

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
