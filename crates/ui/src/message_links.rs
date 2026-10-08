//! Compact message destinations. Resolving and painting consult only accessible cached metadata;
//! activating the URL is left to the existing chat-link navigation path.
use crate::{
	design,
	i18n::{translate, translate_args},
	icons::{self, Icon},
	markdown::ChatLink,
	mentions::MentionSource,
	select::Surface,
};
use egui::{Color32, FontId, Response, Sense, Stroke, StrokeKind, text::LayoutJob};
use std::sync::Arc;

const MAX_NAME_BYTES: usize = 192;
const MAX_NAME_SCAN: usize = 256;
const MAX_CHIP_WIDTH: f32 = 288.0;

pub(crate) struct Presentation {
	name: String,
	icon: Icon,
	pub(crate) tooltip: String,
}

impl Presentation {
	pub(crate) fn resolve(link: &ChatLink, source: Option<&MentionSource<'_>>, url: &str) -> Self {
		let fallback = if link.guild.is_some() {
			"message-link-unknown-channel"
		} else {
			"message-link-unknown-conversation"
		};
		let unknown = || {
			let name = translate(fallback);
			Self {
				name: name.clone(),
				icon: if link.guild.is_some() {
					Icon::Hash
				} else {
					Icon::Profile
				},
				tooltip: format!(
					"{}\n{name}\n{}\n{url}",
					translate("message-link-jump"),
					translate("message-link-unavailable")
				),
			}
		};
		let Some(source) = source else {
			return unknown();
		};
		let state = source.state;
		let Some(channel) = state.channel(link.channel).filter(|channel| {
			channel.guild == link.guild
				&& match link.guild {
					Some(_) => matches!(channel.kind, 0 | 2 | 5 | 10..=12),
					None => matches!(channel.kind, 1 | 3),
				} && state.can_view(channel.id)
				&& state.can_read_history(channel.id)
		}) else {
			// Do not reveal even the server name or channel kind for mismatched/hidden targets.
			return unknown();
		};

		let name = bounded_name(state.conversation_name(channel), fallback);
		let context = if channel.id == source.channel {
			if link.guild.is_some() {
				"message-link-current-channel"
			} else {
				"message-link-current-conversation"
			}
		} else if link.guild.is_some()
			&& state
				.channel(source.channel)
				.is_some_and(|current| current.guild != link.guild)
		{
			"message-link-other-server"
		} else if link.guild.is_some() {
			"message-link-other-channel"
		} else {
			"message-link-other-conversation"
		};
		let mut lines = vec![translate("message-link-jump"), translate(context)];
		if let Some(guild) = link.guild {
			let server = state.guild(guild).map_or_else(
				|| translate("message-link-unknown-server"),
				|guild| bounded_name(&guild.name, "message-link-unknown-server"),
			);
			lines.push(translate_args("message-link-server", &[("name", &server)]));
		}
		let target_key = match channel.kind {
			1 => "message-link-dm",
			3 => "message-link-group",
			10..=12 => "message-link-thread",
			_ => "message-link-channel",
		};
		lines.push(translate_args(target_key, &[("name", &name)]));
		if matches!(channel.kind, 10..=12)
			&& let Some(parent) =
				channel
					.parent_id
					.and_then(|id| state.channel(id))
					.filter(|parent| {
						parent.guild == link.guild
							&& matches!(parent.kind, 0 | 5 | 15 | 16)
							&& state.can_view(parent.id)
							&& state.can_read_history(parent.id)
					}) {
			let parent_name = bounded_name(&parent.name, "message-link-unknown-channel");
			let key = if matches!(parent.kind, 15 | 16) {
				"message-link-forum-parent"
			} else {
				"message-link-parent"
			};
			lines.push(translate_args(key, &[("name", &parent_name)]));
		}
		lines.push(url.to_owned());
		Self {
			name,
			icon: if matches!(channel.kind, 10..=12) {
				Icon::Thread
			} else {
				icons::channel(channel.kind)
			},
			tooltip: lines.join("\n"),
		}
	}

	pub(crate) fn show(&self, ui: &mut egui::Ui, url: &str, surface: &mut Surface) -> Response {
		let colors = design::palette(ui);
		let body_size = egui::TextStyle::Body.resolve(ui.style()).size;
		let height = (body_size + 7.0).ceil();
		let font = FontId::new((body_size - 1.0).max(12.0), design::medium_family(ui.ctx()));
		let icon_size = (height - 6.0).min(18.0);
		let padding = 6.0;
		let chevron_size = 10.0;
		let message_size = 14.0;
		let chrome = padding * 2.0 + icon_size + 4.0 + 4.0 + chevron_size + 2.0 + message_size;
		// Use the full paragraph width, not the remainder of its current row. Egui then
		// wraps the chip as one atom instead of squeezing its name beside preceding text.
		let width_limit = ui.max_rect().width().clamp(1.0, MAX_CHIP_WIDTH);
		let mut job =
			LayoutJob::simple(self.name.clone(), font, colors.mention_text, f32::INFINITY);
		job.break_on_newline = false;
		job.wrap = egui::text::TextWrapping::truncate_at_width((width_limit - chrome).max(0.0));
		let name = ui.fonts_mut(|fonts| fonts.layout_job(job));
		let width = (chrome + name.size().x).min(width_limit);
		let (rect, response) = ui.allocate_exact_size(egui::vec2(width, height), Sense::click());
		let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
		response.widget_info(|| {
			egui::WidgetInfo::labeled(egui::Role::Link, ui.is_enabled(), &self.tooltip)
		});
		if ui.is_rect_visible(rect) {
			let hot = response.hovered() || response.has_focus();
			let fill = if hot || response.is_pointer_button_down_on() {
				colors.selected
			} else {
				colors.mention_bg
			};
			let stroke = if response.has_focus() {
				Stroke::new(2.0, colors.accent)
			} else if hot {
				Stroke::new(1.0, colors.accent)
			} else {
				Stroke::NONE
			};
			let painter = ui.painter().with_clip_rect(ui.clip_rect().intersect(rect));
			painter.rect(rect, 5.0, fill, stroke, StrokeKind::Inside);
			let icon_rect = |x, size| {
				egui::Rect::from_min_size(
					egui::pos2(x, rect.center().y - size / 2.0),
					egui::Vec2::splat(size),
				)
			};
			icons::paint(
				&painter,
				self.icon,
				icon_rect(rect.left() + padding, icon_size),
				colors.mention_text,
			);
			let name_pos = egui::pos2(
				rect.left() + padding + icon_size + 4.0,
				rect.center().y - name.size().y / 2.0,
			);
			let name_right = rect.right() - padding - message_size - 2.0 - chevron_size - 4.0;
			if name_right > name_pos.x {
				painter
					.with_clip_rect(egui::Rect::from_min_max(
						egui::pos2(name_pos.x, rect.top()),
						egui::pos2(name_right, rect.bottom()),
					))
					.galley(name_pos, name, colors.mention_text);
			}
			icons::paint(
				&painter,
				Icon::ChevronRight,
				icon_rect(
					rect.right() - padding - message_size - 2.0 - chevron_size,
					chevron_size,
				),
				colors.mention_text,
			);
			icons::paint(
				&painter,
				Icon::Forum,
				icon_rect(rect.right() - padding - message_size, message_size),
				colors.mention_text,
			);
		}
		// Keep selection in body order and copy the original URL, not the cached name.
		// Like inline emoji, the entire wire text occupies a single unbroken hit slot.
		// Its mesh is empty: the real name/icon visuals above remain independent.
		let galley = selection_galley(ui, url, rect.size());
		surface.run(ui, &response, rect.min, galley, vec![]);
		surface.through(&response);
		response
	}
}

fn selection_galley(ui: &egui::Ui, url: &str, size: egui::Vec2) -> Arc<egui::Galley> {
	let font = egui::TextStyle::Body.resolve(ui.style());
	let mut galley = ui.fonts_mut(|fonts| {
		fonts.layout_job(LayoutJob::simple(
			" ".into(),
			font.clone(),
			Color32::TRANSPARENT,
			f32::INFINITY,
		))
	});
	let galley_mut = Arc::make_mut(&mut galley);
	let placed = &mut galley_mut.rows[0];
	let row = Arc::make_mut(&mut placed.row);
	let slot = row.glyphs[0];
	row.glyphs = url
		.chars()
		.map(|chr| {
			let mut glyph = slot;
			glyph.chr = chr;
			glyph.pos.x = 0.0;
			glyph.advance_width = size.x;
			glyph.line_height = size.y;
			glyph
		})
		.collect();
	row.size = size;
	galley_mut.rect = egui::Rect::from_min_size(egui::Pos2::ZERO, size);
	galley_mut.job = Arc::new(LayoutJob::simple(
		url.into(),
		font,
		Color32::TRANSPARENT,
		f32::INFINITY,
	));
	galley
}

/// Bound both scan work and output. Cached names are untrusted single-line labels, not bidi
/// control programs or tooltip markup. The full URL stays separate from every display name.
fn bounded_name(value: &str, fallback: &str) -> String {
	let mut name = String::new();
	let mut consumed = 0;
	for (at, chr) in value.char_indices().take(MAX_NAME_SCAN) {
		consumed = at + chr.len_utf8();
		if matches!(chr, '\u{061c}' | '\u{200b}'..='\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}' | '\u{feff}')
		{
			continue;
		}
		let chr = if chr.is_whitespace() {
			' '
		} else if chr.is_control() {
			continue;
		} else {
			chr
		};
		if chr == ' ' && (name.is_empty() || name.ends_with(' ')) {
			continue;
		}
		if name.len() + chr.len_utf8() > MAX_NAME_BYTES - '…'.len_utf8() {
			consumed = at;
			break;
		}
		name.push(chr);
	}
	let mut name = name.trim_end().to_owned();
	if name.is_empty() {
		return translate(fallback);
	}
	if consumed < value.len() {
		name.push('…');
	}
	name
}

#[cfg(test)]
mod tests {
	use super::*;
	use model::{Id, permissions as p};

	fn state() -> client_core::State {
		let mut state = test_support::demo_state();
		let base = state.channels[0].clone();
		state.guilds[0].name = "Source server".into();
		let mut other = state.guilds[0].clone();
		other.id = Id(30);
		other.name = "Other server".into();
		state.guilds.push(other);
		state.channels = [
			(20, Some(10), 0, None, "orders"),
			(21, Some(10), 0, None, "project-updates"),
			(22, Some(10), 2, None, "voice-lounge"),
			(23, Some(10), 5, None, "announcements"),
			(24, Some(10), 15, None, "video-forum"),
			(25, Some(10), 11, Some(24), "YT-app"),
			(26, Some(10), 12, Some(21), "private-thread"),
			(27, Some(10), 10, Some(23), "announcement-thread"),
			(28, None, 1, None, "Synthetic Robin"),
			(29, None, 3, None, "Weekend group"),
			(31, Some(30), 0, None, "across-server"),
			(32, Some(10), 4, None, "category-not-a-conversation"),
		]
		.into_iter()
		.map(|(id, guild, kind, parent, name)| model::Channel {
			id: Id(id),
			guild: guild.map(Id),
			kind,
			parent_id: parent.map(Id),
			name: name.into(),
			..base.clone()
		})
		.collect();
		state.invalidate_navigation();
		state
			.permissions
			.replace(test_support::permission_snapshot(&state))
			.unwrap();
		state
	}

	fn resolve(state: &client_core::State, guild: Option<u64>, channel: u64) -> Presentation {
		let link = ChatLink {
			guild: guild.map(Id),
			channel: Id(channel),
			message: Some(Id(100)),
		};
		Presentation::resolve(
			&link,
			Some(&MentionSource {
				state,
				channel: Id(20),
			}),
			"https://discord.com/channels/10/20/100?jump=1#message",
		)
	}

	#[test]
	fn cached_message_destinations_describe_context_and_channel_kinds() {
		let state = state();
		for (guild, channel, icon, context, target) in [
			(
				Some(10),
				20,
				Icon::Hash,
				"Message in this channel",
				"orders",
			),
			(
				Some(10),
				21,
				Icon::Hash,
				"Message in another channel",
				"project-updates",
			),
			(
				Some(10),
				22,
				Icon::Speaker,
				"Message in another channel",
				"voice-lounge",
			),
			(
				Some(10),
				23,
				Icon::Megaphone,
				"Message in another channel",
				"announcements",
			),
			(
				Some(10),
				25,
				Icon::Thread,
				"Message in another channel",
				"YT-app",
			),
			(
				Some(10),
				26,
				Icon::Thread,
				"Message in another channel",
				"private-thread",
			),
			(
				Some(10),
				27,
				Icon::Thread,
				"Message in another channel",
				"announcement-thread",
			),
			(
				None,
				28,
				Icon::Profile,
				"Message in another conversation",
				"Synthetic Robin",
			),
			(
				None,
				29,
				Icon::People,
				"Message in another conversation",
				"Weekend group",
			),
			(
				Some(30),
				31,
				Icon::Hash,
				"Message in another server",
				"across-server",
			),
		] {
			let chip = resolve(&state, guild, channel);
			assert_eq!(chip.name, target);
			assert_eq!(chip.icon, icon);
			assert!(chip.tooltip.contains(context), "{}", chip.tooltip);
			assert!(chip.tooltip.contains(target));
			assert!(chip.tooltip.starts_with("Jump to message\n"));
			assert!(chip.tooltip.ends_with("?jump=1#message"));
			assert!(chip.tooltip.lines().count() <= 6);
		}
		assert!(
			resolve(&state, Some(10), 25)
				.tooltip
				.contains("Forum post in:")
		);
		assert!(
			resolve(&state, Some(10), 25)
				.tooltip
				.contains("video-forum")
		);
		assert!(resolve(&state, Some(10), 26).tooltip.contains("Thread in:"));
		assert!(
			resolve(&state, Some(10), 27)
				.tooltip
				.contains("announcements")
		);
		assert!(
			resolve(&state, None, 28)
				.tooltip
				.contains("Direct message:")
		);
		assert!(
			resolve(&state, None, 29)
				.tooltip
				.contains("Group conversation:")
		);
		assert!(
			resolve(&state, Some(30), 31)
				.tooltip
				.contains("Other server")
		);

		let dm = ChatLink {
			guild: None,
			channel: Id(28),
			message: Some(Id(100)),
		};
		let chip = Presentation::resolve(
			&dm,
			Some(&MentionSource {
				state: &state,
				channel: Id(28),
			}),
			"https://discord.com/channels/@me/28/100",
		);
		assert!(chip.tooltip.contains("Message in this conversation"));
	}

	#[test]
	fn unavailable_or_mismatched_destinations_never_disclose_cached_metadata() {
		let state = state();
		for (guild, channel, fallback) in [
			(Some(10), 999, "Unknown channel"),
			(Some(30), 21, "Unknown channel"),
			(None, 21, "Unknown conversation"),
			(Some(10), 28, "Unknown channel"),
			(Some(10), 32, "Unknown channel"),
		] {
			let chip = resolve(&state, guild, channel);
			assert_eq!(chip.name, fallback);
			assert!(chip.tooltip.contains("unavailable in this session"));
			for secret in [
				"Source server",
				"Other server",
				"project-updates",
				"Synthetic Robin",
				"category-not-a-conversation",
			] {
				assert!(!chip.tooltip.contains(secret), "{}", chip.tooltip);
			}
		}
		for deny in [p::VIEW_CHANNEL, p::READ_MESSAGE_HISTORY] {
			let mut state = self::state();
			let mut snapshot = test_support::permission_snapshot(&state);
			snapshot
				.channels
				.iter_mut()
				.find(|channel| channel.id == Id(21))
				.unwrap()
				.overwrites = Some(vec![p::Overwrite {
				id: Id(10),
				kind: 0,
				allow: 0,
				deny,
			}]);
			state.permissions.replace(snapshot).unwrap();
			for target in [21, 26] {
				let chip = resolve(&state, Some(10), target);
				assert_eq!(chip.name, "Unknown channel");
				assert!(!chip.tooltip.contains("project-updates"));
				assert!(!chip.tooltip.contains("private-thread"));
				assert!(!chip.tooltip.contains("Source server"));
			}
		}
		let link = ChatLink {
			guild: Some(Id(10)),
			channel: Id(20),
			message: Some(Id(100)),
		};
		assert_eq!(
			Presentation::resolve(&link, None, "url").name,
			"Unknown channel"
		);
	}

	#[test]
	fn permission_overwrite_and_member_role_revocations_remove_cached_chip_metadata() {
		use client_core::permissions::Event as PermissionEvent;
		use model::Patch;

		const URL: &str = "https://discord.com/channels/10/26/100?jump=1#message";
		const NAMES: [&str; 3] = ["Source server", "project-updates", "private-thread"];
		fn painted_text(shape: &egui::Shape, text: &mut String) {
			match shape {
				egui::Shape::Text(shape) => {
					text.push_str(shape.galley.text());
					text.push('\n');
				}
				egui::Shape::Vec(shapes) => {
					for shape in shapes {
						painted_text(shape, text);
					}
				}
				_ => {}
			}
		}
		let text = |output: &egui::FullOutput| {
			let mut text = String::new();
			for shape in &output.shapes {
				painted_text(&shape.shape, &mut text);
			}
			text
		};
		let focus_label = |output: &egui::FullOutput| {
			output
				.platform_output
				.events
				.iter()
				.find_map(|event| match event {
					egui::output::OutputEvent::FocusGained(info)
						if info.role == egui::Role::Link =>
					{
						info.label.clone()
					}
					_ => None,
				})
				.expect("focused link accessibility description")
		};
		let tab = |pressed| egui::Event::Key {
			key: egui::Key::Tab,
			physical_key: None,
			pressed,
			repeat: false,
			modifiers: egui::Modifiers::NONE,
		};
		for deny in [p::VIEW_CHANNEL, p::READ_MESSAGE_HISTORY] {
			for member_role in [false, true] {
				let mut state = self::state();
				let mut snapshot = test_support::permission_snapshot(&state);
				if member_role {
					let guild = snapshot
						.guilds
						.iter_mut()
						.find(|guild| guild.id == Id(10))
						.unwrap();
					let roles = guild.roles.as_mut().unwrap();
					let mut role = roles[0].clone();
					role.id = Id(1000);
					role.name = "Revocable link access".into();
					role.bits = deny;
					roles[0].bits &= !deny;
					roles.push(role);
					guild.member.as_mut().unwrap().roles.push(Id(1000));
				}
				state.permissions.replace(snapshot).unwrap();
				let ctx = egui::Context::default();
				ctx.all_styles_mut(|style| {
					style.interaction.tooltip_delay = 0.0;
					style.interaction.tooltip_grace_time = 0.0;
				});
				let link = ChatLink {
					guild: Some(Id(10)),
					channel: Id(26),
					message: Some(Id(100)),
				};
				let mut clock = 0.0;
				let mut frame = |state: &client_core::State, events| {
					clock += 1.0;
					let mut hit = None;
					let output = ctx.run_ui(
						egui::RawInput {
							screen_rect: Some(egui::Rect::from_min_size(
								egui::Pos2::ZERO,
								egui::vec2(500.0, 360.0),
							)),
							time: Some(clock),
							events,
							..Default::default()
						},
						|ui| {
							let chip = Presentation::resolve(
								&link,
								Some(&MentionSource {
									state,
									channel: Id(20),
								}),
								URL,
							);
							let mut surface = Surface::new(ui, "revoked-message-link");
							let response = chip
								.show(ui, URL, &mut surface)
								.on_hover_text(&chip.tooltip);
							assert!(!response.clicked());
							hit = Some((response.id, response.rect));
							surface.finish(ui);
						},
					);
					assert!(output.platform_output.commands.is_empty());
					let (id, rect) = hit.unwrap();
					(output, id, rect)
				};
				// Warm both parent and inherited thread decisions before changing permissions.
				for target in [21, 26] {
					assert!(state.can_view(Id(target)) && state.can_read_history(Id(target)));
				}
				for _ in 0..3 {
					frame(&state, vec![]).0.drop_without_applying_deltas();
				}
				let (output, id, rect) = frame(&state, vec![tab(true)]);
				let accessible = focus_label(&output);
				for name in NAMES {
					assert!(accessible.contains(name), "{accessible}");
				}
				assert!(text(&output).contains("private-thread"));
				output.drop_without_applying_deltas();
				let point = rect.center();
				for pass in 0..3 {
					let (output, _, _) =
						frame(&state, vec![tab(false), egui::Event::PointerMoved(point)]);
					if pass == 2 {
						let tooltip = text(&output);
						assert!(tooltip.contains("Jump to message"));
						for name in NAMES {
							assert!(tooltip.contains(name), "{tooltip}");
						}
					}
					output.drop_without_applying_deltas();
				}
				let event = if member_role {
					PermissionEvent::Member {
						guild: Id(10),
						roles: Patch::Value(vec![]),
						timeout_until: Patch::Absent,
					}
				} else {
					PermissionEvent::Channel {
						channel: Id(21),
						guild: Some(Id(10)),
						overwrites: Patch::Value(vec![p::Overwrite {
							id: Id(10),
							kind: 0,
							allow: 0,
							deny,
						}]),
					}
				};
				state.apply(client_core::Envelope {
					generation: state.generation,
					event: client_core::Event::Permissions(event),
				});
				// Names deliberately remain cached: the permission gate, not deletion, scrubs them.
				assert_eq!(state.guild(Id(10)).unwrap().name, NAMES[0]);
				assert_eq!(state.channel(Id(21)).unwrap().name, NAMES[1]);
				assert_eq!(state.channel(Id(26)).unwrap().name, NAMES[2]);
				for target in [21, 26] {
					assert!(!state.can_read_history(Id(target)));
					let chip = resolve(&state, Some(10), target);
					assert_eq!(chip.name, "Unknown channel");
					assert!(chip.tooltip.contains("unavailable in this session"));
					for name in NAMES {
						assert!(!chip.tooltip.contains(name));
					}
				}
				for pass in 0..3 {
					let (output, _, _) = frame(&state, vec![egui::Event::PointerMoved(point)]);
					let tooltip = text(&output);
					assert!(tooltip.contains("Unknown channel"));
					for name in NAMES {
						assert!(!tooltip.contains(name), "{tooltip}");
					}
					if pass == 2 {
						assert!(tooltip.contains("Jump to message"));
						assert!(
							tooltip.contains("unavailable in this session")
								&& tooltip.contains(URL)
						);
					}
					output.drop_without_applying_deltas();
				}
				ctx.memory_mut(|memory| memory.surrender_focus(id));
				frame(&state, vec![egui::Event::PointerGone])
					.0
					.drop_without_applying_deltas();
				let (output, _, _) = frame(&state, vec![tab(true)]);
				let accessible = focus_label(&output);
				assert!(accessible.contains("Unknown channel") && accessible.contains(URL));
				for name in NAMES {
					assert!(!accessible.contains(name), "{accessible}");
				}
				output.drop_without_applying_deltas();
			}
		}
	}

	#[test]
	fn cached_labels_are_bounded_single_line_and_strip_spoofing_controls() {
		assert_eq!(
			bounded_name(
				"\u{202e}  orders\n\t desk\u{200b}\u{0}",
				"message-link-unknown-channel"
			),
			"orders desk"
		);
		assert_eq!(
			bounded_name(" \u{202e}\n", "message-link-unknown-channel"),
			"Unknown channel"
		);
		for value in [
			"界🦀".repeat(400),
			format!("{}\nsecret", "\u{202e}".repeat(500)),
		] {
			let name = bounded_name(&value, "message-link-unknown-channel");
			assert!(name.len() <= MAX_NAME_BYTES);
			assert!(!name.contains(['\n', '\u{202e}']));
		}
		assert!(bounded_name(&"界".repeat(400), "message-link-unknown-channel").ends_with('…'));
	}

	#[test]
	fn long_chips_ellipsize_and_wrap_as_one_bounded_atom() {
		for width in [80.0, 180.0, 600.0] {
			let ctx = egui::Context::default();
			let chip = Presentation {
				name: "a-very-long-conversation-name-".repeat(6),
				icon: Icon::Thread,
				tooltip: "Jump to message".into(),
			};
			let mut rect = egui::Rect::NOTHING;
			let mut prefix = egui::Rect::NOTHING;
			let output = ctx.run_ui(
				egui::RawInput {
					screen_rect: Some(egui::Rect::from_min_size(
						egui::Pos2::ZERO,
						egui::vec2(width, 200.0),
					)),
					..Default::default()
				},
				|ui| {
					let mut surface = Surface::new(ui, "chip");
					ui.allocate_ui_with_layout(
						egui::vec2(ui.available_width(), 0.0),
						egui::Layout::left_to_right(egui::Align::Min).with_main_wrap(true),
						|ui| {
							ui.spacing_mut().item_spacing = egui::Vec2::ZERO;
							prefix = ui.label("Before ").rect;
							rect = chip
								.show(ui, "https://discord.com/channels/10/20/100", &mut surface)
								.rect;
						},
					);
					surface.finish(ui);
				},
			);
			assert!(rect.width() > 0.0 && rect.width() <= MAX_CHIP_WIDTH);
			assert!(rect.right() <= width + 0.1 && rect.left() >= 0.0);
			if width < MAX_CHIP_WIDTH {
				assert!(rect.top() > prefix.top(), "{width}: {rect:?}");
			}
			let text = output
				.shapes
				.iter()
				.find_map(|shape| match &shape.shape {
					egui::Shape::Text(text) if text.galley.text() == chip.name => Some(text),
					_ => None,
				})
				.expect("painted bounded channel name");
			assert_eq!(text.galley.rows.len(), 1);
			assert!(text.galley.elided);
			output.drop_without_applying_deltas();
		}
	}

	#[test]
	fn url_selection_geometry_is_atomic_and_uses_body_sized_blank_mesh() {
		let ctx = egui::Context::default();
		let url = format!(
			"HTTPS://DISCORD.COM:443/channels/10/20/100?q={}#message",
			"x".repeat(1900)
		);
		let output = ctx.run_ui(Default::default(), |ui| {
			let size = egui::vec2(140.0, 22.0);
			let galley = selection_galley(ui, &url, size);
			assert_eq!(galley.text(), url);
			assert_eq!(galley.size(), size);
			assert_eq!(galley.rows.len(), 1);
			assert_eq!(galley.rows[0].glyphs.len(), url.len());
			assert!(galley.rows[0].visuals.mesh.is_empty());
			assert_eq!(galley.cursor_from_pos(egui::vec2(35.0, 10.0)).index.0, 0);
			assert_eq!(
				galley.cursor_from_pos(egui::vec2(105.0, 10.0)).index.0,
				url.len()
			);
			assert!(
				galley
					.job
					.sections
					.iter()
					.all(|section| section.format.font_id.size <= 20.0)
			);
		});
		output.drop_without_applying_deltas();
	}
}
