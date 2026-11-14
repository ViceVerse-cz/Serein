//! Incoming typing uses existing conversation identities, never profile or directory requests.
use client_core::State;
use model::Id;
use std::time::Instant;

/// The name this conversation shows for a typist: a loaded guild nickname still wins, and the
/// identities the channel and its messages already carry are shown under the local friend
/// nickname, so the indicator names a DM the way its title and message authors do.
fn name(state: &State, channel: Id, user: Id) -> Option<&str> {
	state
		.channels
		.iter()
		.find(|entry| entry.id == channel)
		.and_then(|entry| entry.recipients.iter().find(|entry| entry.id == user))
		.map(|recipient| state.user_display_name(recipient))
		.or_else(|| {
			state
				.members
				.as_ref()
				.filter(|members| members.channel == channel)
				.and_then(|members| {
					members
						.slots
						.iter()
						.flatten()
						.filter_map(|slot| match slot {
							model::MemberSlot::Person(m) => Some(m),
							_ => None,
						})
						.find(|member| member.user.id == user)
				})
				.map(|member| {
					member
						.nick
						.as_deref()
						.filter(|nick| !nick.is_empty())
						.unwrap_or_else(|| state.user_display_name(&member.user))
				})
		})
		.or_else(|| {
			// A message this person wrote names them the way their author label does, so it wins
			// over a mention of them, which only carries the display name.
			let matching = || {
				state
					.timeline
					.iter()
					.filter(|message| message.channel == channel)
			};
			matching()
				.find(|message| message.author.id == user)
				.map(|message| state.message_author_name(message))
				.or_else(|| {
					matching().find_map(|message| {
						message
							.mentions
							.iter()
							.find(|entry| entry.id == user)
							.map(|user| state.user_display_name(user))
					})
				})
		})
}

/// Text runs for the indicator; `true` marks a typist name rendered in the strong weight.
fn segments(state: &State, channel: Id, now: Instant) -> Option<Vec<(String, bool)>> {
	if state.selected != Some(channel) {
		return None;
	}
	let mut names = Vec::new();
	let mut count = 0;
	for user in state.typing_users(now) {
		count += 1;
		if names.len() < 3
			&& let Some(name) = name(state, channel, user)
		{
			let name: String = name
				.chars()
				.take(40)
				.map(|c| if c.is_control() { ' ' } else { c })
				.collect();
			if !name.trim().is_empty() {
				names.push(name);
			}
		}
	}
	if count == 0 {
		return None;
	}
	let verb = if typing_tweaks_enabled() {
		if count == 1 {
			" is typing (typingTweaks active)…"
		} else {
			" are typing (typingTweaks active)…"
		}
	} else if count == 1 {
		" is typing…"
	} else {
		" are typing…"
	};
	if names.is_empty() {
		return Some(vec![(
			if count == 1 {
				"Someone is typing…".into()
			} else {
				format!("{count} people are typing…")
			},
			false,
		)]);
	}
	let others = count - names.len();
	let mut out: Vec<(String, bool)> = Vec::with_capacity(names.len() * 2 + 2);
	let last_index = names.len() - 1 + usize::from(others > 0);
	for (index, name) in names.into_iter().enumerate() {
		if index > 0 {
			out.push((
				if index == last_index { " and " } else { ", " }.into(),
				false,
			));
		}
		out.push((name, true));
	}
	if others > 0 {
		out.push((
			format!(" and {others} other{}", if others == 1 { "" } else { "s" }),
			false,
		));
	}
	out.push((verb.into(), false));
	Some(out)
}

#[cfg(test)]
fn label(state: &State, channel: Id, now: Instant) -> Option<String> {
	segments(state, channel, now).map(|parts| parts.into_iter().map(|(text, _)| text).collect())
}

/// Height the timeline reserves for the indicator above the composer. The conversation keeps
/// the gap on idle frames, so typing never moves messages.
pub(super) const OVERLAY_HEIGHT: f32 = 22.0;
const TEXT_SIZE: f32 = 12.5;
const DOT_RADIUS: f32 = 2.5;
const DOT_STEP: f32 = 7.0;
const DOT_PERIOD: f64 = 1.2;

/// Whether the indicator has anything to say for this conversation right now.
pub(super) fn active(state: &State, channel: Id, now: Instant) -> bool {
	segments(state, channel, now).is_some()
}

/// Paints the indicator into `rect`, which the timeline reserves above the composer.
pub(super) fn overlay(
	ui: &mut egui::Ui,
	rect: egui::Rect,
	state: &State,
	channel: Id,
	now: Instant,
) {
	let colors = crate::design::palette(ui);
	let Some(segments) = segments(state, channel, now) else {
		return;
	};
	if !ui.is_rect_visible(rect) {
		return;
	}
	let response = ui.interact(
		rect,
		ui.make_persistent_id("typing-indicator"),
		egui::Sense::hover(),
	);
	let painter = ui.painter_at(rect);
	let no_animation = NO_TYPING_ANIMATION_ENABLED.load(std::sync::atomic::Ordering::Relaxed);
	// Three pulsing dots, Discord style; the animation runs only while someone is typing and
	// this window has focus. A background window, or a static-indicator tweak, paints them at
	// rest and asks for frames only at expiry.
	let animate = ui.input(|input| input.focused)
		&& !no_animation
		&& !crate::interface_tweaks::on(
			ui.ctx(),
			extensions::InterfaceTweak::StaticTypingIndicator,
		);
	let time = ui.input(|input| input.time);
	let dots_left = rect.left() + DOT_RADIUS + 2.0;
	for index in 0..3 {
		let phase = (time / DOT_PERIOD - f64::from(index) * 0.18).fract();
		let pulse = if animate {
			(0.5 - 0.5 * (phase * std::f64::consts::TAU).cos()) as f32
		} else {
			0.5
		};
		let center = egui::pos2(dots_left + index as f32 * DOT_STEP, rect.center().y);
		painter.circle_filled(
			center,
			DOT_RADIUS - 0.5 + pulse * 0.6,
			colors.muted.gamma_multiply(0.35 + 0.65 * pulse),
		);
	}
	let text_left = dots_left + 2.0 * DOT_STEP + DOT_RADIUS + 8.0;
	let mut job = egui::text::LayoutJob::default();
	job.wrap.max_width = (rect.right() - text_left).max(0.0);
	job.wrap.max_rows = 1;
	job.wrap.break_anywhere = true;
	if typing_indicator_avatar_enabled() {
		painter.circle_filled(
			egui::pos2(dots_left - 1.0, rect.center().y),
			2.0,
			colors.accent,
		);
	}
	let strong = crate::design::semibold_family(ui.ctx());
	for (text, is_name) in &segments {
		job.append(
			text,
			0.0,
			egui::TextFormat {
				font_id: egui::FontId::new(
					TEXT_SIZE,
					if *is_name {
						strong.clone()
					} else {
						egui::FontFamily::Proportional
					},
				),
				color: if *is_name {
					colors.text_strong
				} else {
					colors.muted
				},
				..Default::default()
			},
		);
	}
	let galley = ui.fonts_mut(|fonts| fonts.layout_job(job));
	let text_pos = egui::pos2(text_left, rect.center().y - galley.size().y / 2.0);
	painter.galley(text_pos, galley, colors.muted);
	response.widget_info(|| {
		let text: String = segments.iter().map(|(text, _)| text.as_str()).collect();
		egui::WidgetInfo::labeled(egui::Role::Label, true, text)
	});
	if let Some(deadline) = state.typing_deadline(now) {
		// Keep the dots moving until the earliest deadline, then go idle without repaints.
		// Unfocused, only the expiry itself is worth a frame; the dots hold still until then.
		let frame = std::time::Duration::from_millis(80);
		let delay = deadline.saturating_duration_since(now);
		ui.ctx()
			.request_repaint_after(if animate { delay.min(frame) } else { delay });
	}
}

pub static NO_TYPING_ANIMATION_ENABLED: std::sync::atomic::AtomicBool =
	std::sync::atomic::AtomicBool::new(false);

#[allow(dead_code)]
pub fn set_no_typing_animation_enabled(enabled: bool) {
	NO_TYPING_ANIMATION_ENABLED.store(enabled, std::sync::atomic::Ordering::Relaxed);
}

pub static TYPING_TWEAKS_ENABLED: std::sync::atomic::AtomicBool =
	std::sync::atomic::AtomicBool::new(false);

pub fn typing_tweaks_enabled() -> bool {
	TYPING_TWEAKS_ENABLED.load(std::sync::atomic::Ordering::Relaxed)
}

pub fn set_typing_tweaks_enabled(enabled: bool) {
	TYPING_TWEAKS_ENABLED.store(enabled, std::sync::atomic::Ordering::Relaxed);
}

pub static TYPING_INDICATOR_AVATAR_ENABLED: std::sync::atomic::AtomicBool =
	std::sync::atomic::AtomicBool::new(false);

pub fn typing_indicator_avatar_enabled() -> bool {
	TYPING_INDICATOR_AVATAR_ENABLED.load(std::sync::atomic::Ordering::Relaxed)
}

pub fn set_typing_indicator_avatar_enabled(enabled: bool) {
	TYPING_INDICATOR_AVATAR_ENABLED.store(enabled, std::sync::atomic::Ordering::Relaxed);
}

#[cfg(test)]
mod tests {
	#[test]
	fn typing_tweaks_and_avatar_toggles_work() {
		assert!(!typing_tweaks_enabled());
		set_typing_tweaks_enabled(true);
		assert!(typing_tweaks_enabled());
		set_typing_tweaks_enabled(false);

		assert!(!typing_indicator_avatar_enabled());
		set_typing_indicator_avatar_enabled(true);
		assert!(typing_indicator_avatar_enabled());
		set_typing_indicator_avatar_enabled(false);
	}

	#[test]
	fn no_typing_animation_toggle_works() {
		set_no_typing_animation_enabled(true);
		assert!(NO_TYPING_ANIMATION_ENABLED.load(std::sync::atomic::Ordering::Relaxed));
		set_no_typing_animation_enabled(false);
		assert!(!NO_TYPING_ANIMATION_ENABLED.load(std::sync::atomic::Ordering::Relaxed));
	}

	use super::*;
	use client_core::{auth::AuthState, typing::Signal};
	use model::{Channel, Freshness, User};
	use std::time::{Duration, SystemTime};

	fn state() -> State {
		State {
			auth: AuthState::Authenticated,
			gateway_connected: true,
			freshness: Freshness::Fresh,
			selected: Some(Id(10)),
			user: Some(User {
				id: Id(99),
				name: "You".into(),
				avatar: None,
				webhook: false,
				kind: Default::default(),
				discriminator: 0,
				primary_guild: None,
				cosmetics: None,
			}),
			channels: vec![Channel {
				user_limit: None,
				id: Id(10),
				guild: None,
				parent_id: None,
				kind: 1,
				position: 0,
				name: "Synthetic typing conversation".into(),
				last_message: None,
				member_list_id: None,
				tags: None,
				message_count: None,
				icon: None,
				recipients: vec![
					User {
						id: Id(1),
						name: "Alex".into(),
						avatar: None,
						webhook: false,
						kind: Default::default(),
						discriminator: 0,
						primary_guild: None,
						cosmetics: None,
					},
					User {
						id: Id(2),
						name: "Robin".into(),
						avatar: None,
						webhook: false,
						kind: Default::default(),
						discriminator: 0,
						primary_guild: None,
						cosmetics: None,
					},
					User {
						id: Id(3),
						name: "Long name\n".repeat(100),
						avatar: None,
						webhook: false,
						kind: Default::default(),
						discriminator: 0,
						primary_guild: None,
						cosmetics: None,
					},
				],
			}],
			..Default::default()
		}
	}

	#[test]
	fn bounded_names_and_expiry_render_without_idle_repaints_or_scope_leaks() {
		let now = Instant::now();
		let wall = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000);
		let mut state = state();
		for user in 1..=8 {
			state.observe_typing_at(
				Signal {
					channel: Id(10),
					user: Id(user),
					timestamp: 1_000,
				},
				wall,
				now,
			);
		}
		let text = label(&state, Id(10), now).unwrap();
		assert!(text.starts_with("Alex, Robin, Long name"));
		assert!(text.ends_with("and 5 others are typing…"));
		assert!(!text.contains('\n'));
		assert!(text.chars().count() < 160);
		assert!(label(&state, Id(11), now).is_none());
		for (width, focused) in [(160.0, true), (760.0, true), (760.0, false)] {
			let ctx = egui::Context::default();
			let mut row_height = None;
			for expired in [false, true] {
				let instant = if expired {
					now + Duration::from_secs(11)
				} else {
					now
				};
				for pass in 0..5 {
					let output = ctx.run_ui(
						egui::RawInput {
							focused,
							screen_rect: Some(egui::Rect::from_min_size(
								egui::Pos2::ZERO,
								egui::vec2(width, 100.0),
							)),
							..Default::default()
						},
						|ui| {
							let rect = egui::Rect::from_min_size(
								egui::pos2(0.0, 100.0 - OVERLAY_HEIGHT),
								egui::vec2(width, OVERLAY_HEIGHT),
							);
							overlay(ui, rect, &state, Id(10), instant);
							assert!(rect.width() <= width);
							assert_eq!(*row_height.get_or_insert(rect.height()), rect.height());
						},
					);
					assert!(output.platform_output.commands.is_empty());
					let rendered = output.shapes.iter().any(|shape| {
						matches!(&shape.shape,
                        egui::Shape::Text(text) if text.galley.job.text.contains("typing"))
					});
					assert_eq!(rendered, !expired);
					if pass == 4 {
						let delay = output.viewport_output[&egui::ViewportId::ROOT].repaint_delay;
						if expired {
							assert_eq!(delay, Duration::MAX);
						} else if focused {
							// Animation frames only while the window has focus.
							assert!(delay > Duration::ZERO && delay <= Duration::from_millis(80));
						} else {
							assert!(
								delay > Duration::from_millis(80)
									&& delay <= Duration::from_secs(10),
								"an unfocused window must not ask for animation frames"
							);
						}
					}
					output.drop_without_applying_deltas();
				}
			}
		}
		state.gateway_connected = false;
		assert!(label(&state, Id(10), now).is_none());
		state.gateway_connected = true;
		state.selected = Some(Id(11));
		assert!(label(&state, Id(10), now).is_none());
		state.selected = Some(Id(10));
		state.freshness = Freshness::Stale;
		assert!(label(&state, Id(10), now).is_none());
	}

	/// Checks that the static-indicator tweak keeps a focused window's dots still and asks
	/// for no animation frames, in the chat indicator and the avatar typing badge.
	#[test]
	fn static_typing_tweak_stills_dots_without_animation_frames() {
		let now = Instant::now();
		let mut state = state();
		state.observe_typing_at(
			Signal {
				channel: Id(10),
				user: Id(1),
				timestamp: 1_000,
			},
			SystemTime::UNIX_EPOCH + Duration::from_secs(1_000),
			now,
		);
		let tweaks = |on: bool| {
			std::sync::Arc::new(
				on.then_some(extensions::InterfaceTweak::StaticTypingIndicator)
					.into_iter()
					.collect(),
			)
		};
		let run = |ctx: &egui::Context, time: f64, on: bool| {
			let output = ctx.run_ui(
				egui::RawInput {
					focused: true,
					time: Some(time),
					screen_rect: Some(egui::Rect::from_min_size(
						egui::Pos2::ZERO,
						egui::vec2(400.0, 100.0),
					)),
					..Default::default()
				},
				|ui| {
					crate::interface_tweaks::publish(ui.ctx(), &tweaks(on));
					let rect = egui::Rect::from_min_size(
						egui::pos2(0.0, 100.0 - OVERLAY_HEIGHT),
						egui::vec2(400.0, OVERLAY_HEIGHT),
					);
					overlay(ui, rect, &state, Id(10), now);
					crate::profiles::typing_badge(
						ui,
						egui::Rect::from_min_size(egui::pos2(300.0, 0.0), egui::vec2(40.0, 40.0)),
						Some("online"),
						egui::Color32::BLACK,
					);
				},
			);
			let delay = output.viewport_output[&egui::ViewportId::ROOT].repaint_delay;
			let dots: Vec<String> = output
				.shapes
				.iter()
				.filter(|shape| matches!(shape.shape, egui::Shape::Circle(_)))
				.map(|shape| format!("{:?}", shape.shape))
				.collect();
			output.drop_without_applying_deltas();
			(delay, dots)
		};
		for on in [false, true] {
			let ctx = egui::Context::default();
			// The first two frames always ask for another; later ones show the steady request.
			run(&ctx, 0.0, on);
			run(&ctx, 0.05, on);
			let (_, first) = run(&ctx, 0.1, on);
			let (delay, later) = run(&ctx, 0.5, on);
			assert_eq!(first.len(), 6, "three chat dots and three badge dots");
			if on {
				assert_eq!(first, later, "still dots do not change over time");
				assert!(
					delay >= Duration::from_millis(1000),
					"no animation frames: {delay:?}"
				);
			} else {
				assert_ne!(first, later, "dots pulse without the tweak");
				assert!(delay <= Duration::from_millis(80));
			}
		}
	}

	#[test]
	fn a_local_friend_nickname_names_the_typist() {
		let now = Instant::now();
		let wall = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000);
		let mut state = state();
		let friend = state.channels[0].recipients[0].clone();
		let apply = |state: &mut State, event: client_core::user_actions::Event| {
			state.apply(client_core::Envelope {
				generation: state.generation,
				event: client_core::Event::UserAction(event),
			});
		};
		apply(
			&mut state,
			client_core::user_actions::Event::Friends(Some(vec![(
				friend.clone(),
				"synthetic.friend".into(),
			)])),
		);
		apply(
			&mut state,
			client_core::user_actions::Event::Nickname {
				user: friend.id,
				text: "Bestie".into(),
			},
		);
		assert_eq!(state.conversation_name(&state.channels[0]), "Bestie");
		let signal = Signal {
			channel: Id(10),
			user: friend.id,
			timestamp: 1_000,
		};
		state.observe_typing_at(signal, wall, now);
		assert_eq!(
			label(&state, Id(10), now).as_deref(),
			Some("Bestie is typing…")
		);
		// A cleared nickname falls back to the name the conversation otherwise shows.
		apply(
			&mut state,
			client_core::user_actions::Event::Nickname {
				user: friend.id,
				text: String::new(),
			},
		);
		assert_eq!(
			label(&state, Id(10), now).as_deref(),
			Some("Alex is typing…")
		);
	}

	#[test]
	fn a_message_the_person_wrote_names_them_before_one_that_merely_mentions_them() {
		let mut state = state();
		let channel = Id(60);
		let mut typist = state.channels[0].recipients[0].clone();
		typist.id = Id(7);
		typist.name = "Synthetic typist".into();
		state.channels.push(Channel {
			user_limit: None,
			id: channel,
			guild: Some(Id(50)),
			parent_id: None,
			kind: 0,
			position: 0,
			name: "synthetic-channel".into(),
			last_message: None,
			member_list_id: None,
			tags: None,
			message_count: None,
			icon: None,
			recipients: Vec::new(),
		});
		// The typist is a friend with a private nickname; the member window stays unloaded.
		let apply = |state: &mut State, event: client_core::user_actions::Event| {
			state.apply(client_core::Envelope {
				generation: state.generation,
				event: client_core::Event::UserAction(event),
			});
		};
		apply(
			&mut state,
			client_core::user_actions::Event::Relationships(Some(vec![])),
		);
		apply(
			&mut state,
			client_core::user_actions::Event::Friends(Some(vec![(
				typist.clone(),
				"synthetic.typist".into(),
			)])),
		);
		apply(
			&mut state,
			client_core::user_actions::Event::Nickname {
				user: typist.id,
				text: "Private name".into(),
			},
		);
		// An earlier message only mentions them; a later one is theirs and carries a server
		// nickname, which is what their author label shows.
		let mut mention = test_support::message(1, channel);
		mention.author.id = Id(8);
		mention.mentions = vec![typist.clone()];
		state.timeline.insert(mention, false, false).unwrap();
		let mut authored = test_support::message(2, channel);
		authored.author = typist.clone();
		authored.author_nick = Some("Server nick".into());
		state.timeline.insert(authored, false, false).unwrap();
		assert_eq!(state.user_display_name(&typist), "Private name");
		assert_eq!(name(&state, channel, typist.id), Some("Server nick"));
	}

	#[test]
	fn unknown_typist_uses_generic_label_and_composer_keeps_draft_without_commands() {
		let now = Instant::now();
		let wall = SystemTime::now();
		let mut state = state();
		state.demo = true;
		let timestamp = wall
			.duration_since(SystemTime::UNIX_EPOCH)
			.unwrap()
			.as_secs();
		state.observe_typing_at(
			Signal {
				channel: Id(10),
				user: Id(8),
				timestamp,
			},
			wall,
			now,
		);
		assert_eq!(
			label(&state, Id(10), now).as_deref(),
			Some("Someone is typing…")
		);
		state.drafts.insert(Id(10), "My unsent draft".into());
		let mut messaging = crate::MessagingUi::default();
		let ctx = egui::Context::default();
		for pass in 0..3 {
			let mut commands = vec![];
			let mut output = ctx.run_ui(
				egui::RawInput {
					screen_rect: Some(egui::Rect::from_min_size(
						egui::Pos2::ZERO,
						egui::vec2(900.0, 500.0),
					)),
					..Default::default()
				},
				|ui| commands.extend(messaging.show(ui, &mut state)),
			);
			output.textures_delta.clear();
			assert!(commands.is_empty());
			assert!(output.platform_output.commands.is_empty());
			if pass > 0 {
				assert!(output.shapes.iter().any(|shape| matches!(&shape.shape,
                    egui::Shape::Text(text) if text.galley.job.text == "Someone is typing…")));
			}
			assert_eq!(state.drafts[&Id(10)], "My unsent draft");
			assert!(messaging.draft_changes.is_empty());
			output.drop_without_applying_deltas();
		}
	}
}
