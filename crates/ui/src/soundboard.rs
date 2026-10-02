//! Explicit native sound selection; no local preview download or playback.
use crate::{design, dialog};
use client_core::{
	Command, State,
	soundboard::{Action, Scope},
	voice::Phase,
};
use egui::RichText;

#[derive(Default)]
pub struct SoundboardUi {
	open: Option<Scope>,
}
fn scope(state: &State) -> Option<Scope> {
	if !state.demo {
		return state.soundboard_scope();
	}
	let call = state.voice.active.as_ref()?;
	let guild = call.guild?;
	(matches!(call.phase, Phase::Connected | Phase::Waiting)
		&& state
			.channel(call.channel)
			.is_some_and(|c| c.kind == 2 && c.guild == Some(guild)))
	.then_some(Scope {
		generation: state.generation,
		channel: call.channel,
		guild,
		call_request: call.request,
	})
}
impl SoundboardUi {
	pub fn available(state: &State) -> bool {
		scope(state).is_some()
	}
	pub fn open(&mut self, state: &mut State, commands: &mut Vec<Command>) {
		let Some(current) = scope(state) else {
			return;
		};
		self.open = Some(current);
		if state.soundboard.scope != Some(current)
			&& let Some(command) = state.request_soundboard(Action::Load)
		{
			commands.push(command);
		}
	}
	pub(super) fn show(
		&mut self,
		ctx: &egui::Context,
		state: &mut State,
		commands: &mut Vec<Command>,
	) {
		if !state.demo {
			state.revalidate_soundboard();
		}
		let Some(open) = self.open else {
			return;
		};
		if scope(state) != Some(open) {
			self.open = None;
			return;
		}
		let mut close = false;
		let mut action = None;
		let modal = dialog::Dialog::new(
			"voice-soundboard",
			crate::i18n::translate("soundboard-heading"),
		)
		.subtitle(crate::i18n::translate("soundboard-subtitle"))
		.width(420.0)
		.show(ctx, |body| {
			body.scroll(120.0, |ui| {
				let colors = design::palette(ui);
				if let Some(channel) = state.channel(open.channel) {
					ui.label(crate::i18n::translate_args(
						"soundboard-channel",
						&[("channel", &channel.name)],
					));
				}
				if state.demo {
					ui.colored_label(colors.warning, crate::i18n::translate("soundboard-offline"));
					for (heading, sounds) in [
						("soundboard-default", &["🦆 Quack", "👏 Applause"][..]),
						("soundboard-guild", &["🎉 Synthetic celebration"][..]),
					] {
						ui.label(RichText::new(crate::i18n::translate(heading)).strong());
						for sound in sounds {
							ui.add_enabled(false, egui::Button::new(*sound));
						}
					}
				} else {
					if state.soundboard.pending.is_some() {
						ui.label(crate::i18n::translate("soundboard-pending"));
					}
					if let Some(error) = state.soundboard.error {
						ui.colored_label(colors.danger, crate::i18n::translate(error_key(error)));
					}
					if state.soundboard.scope == Some(open) && state.soundboard.sounds.is_empty() {
						ui.label(crate::i18n::translate("soundboard-empty"));
					}
					for guild in [None, Some(open.guild)] {
						if !state
							.soundboard
							.sounds
							.iter()
							.any(|sound| sound.guild == guild)
						{
							continue;
						}
						ui.label(
							RichText::new(crate::i18n::translate(if guild.is_some() {
								"soundboard-guild"
							} else {
								"soundboard-default"
							}))
							.strong(),
						);
						for sound in state
							.soundboard
							.sounds
							.iter()
							.filter(|sound| sound.guild == guild)
						{
							let label = sound.emoji.as_ref().map_or_else(
								|| sound.name.clone(),
								|emoji| format!("{emoji} {}", sound.name),
							);
							let enabled = state.can_play_soundboard(sound.id);
							if ui
								.add_enabled(enabled, egui::Button::new(label))
								.on_hover_text(crate::i18n::translate(if sound.available {
									"soundboard-play"
								} else {
									"soundboard-unavailable"
								}))
								.clicked()
							{
								action = Some(Action::Play(sound.id));
							}
						}
					}
				}
			});
			body.footer(|ui| {
				close = dialog::action(ui, "soundboard-close", dialog::Action::Neutral).clicked();
				if ui
					.add_enabled_ui(!state.demo && state.soundboard.pending.is_none(), |ui| {
						dialog::action(ui, "soundboard-refresh", dialog::Action::Neutral)
					})
					.inner
					.clicked()
				{
					action = Some(Action::Load);
				}
			});
		});
		if let Some(action) = action
			&& let Some(command) = state.request_soundboard(action)
		{
			commands.push(command);
		}
		if close || modal.close {
			self.open = None;
		}
		ctx.request_repaint_after(std::time::Duration::from_secs(1));
	}
}

fn error_key(error: client_core::auth::Failure) -> &'static str {
	use client_core::auth::Failure;
	match error {
		Failure::Forbidden => "soundboard-error-permission",
		Failure::RateLimited => "soundboard-error-rate",
		Failure::Ambiguous => "soundboard-error-ambiguous",
		Failure::Network => "soundboard-error-network",
		Failure::Protocol => "soundboard-error-response",
		Failure::ProtocolAt(_) => "soundboard-error-unavailable",
		Failure::Capacity | Failure::CapacityAt(_) => "soundboard-error-capacity",
		Failure::Expired | Failure::Challenged | Failure::InvalidCredential => {
			"soundboard-error-session"
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use model::Id;
	fn frame(
		ctx: &egui::Context,
		view: &mut SoundboardUi,
		state: &mut State,
		commands: &mut Vec<Command>,
		events: Vec<egui::Event>,
	) -> Vec<(String, egui::Rect)> {
		let output = ctx.run_ui(
			egui::RawInput {
				screen_rect: Some(egui::Rect::from_min_size(
					egui::Pos2::ZERO,
					egui::vec2(760.0, 520.0),
				)),
				events,
				..Default::default()
			},
			|_| view.show(ctx, state, commands),
		);
		let labels = output
			.shapes
			.iter()
			.filter_map(|shape| match &shape.shape {
				egui::Shape::Text(text) => Some((
					text.galley.text().to_owned(),
					egui::Rect::from_min_size(text.pos, text.galley.size()),
				)),
				_ => None,
			})
			.collect();
		output.drop_without_applying_deltas();
		labels
	}
	#[test]
	fn soundboard_load_and_play_require_explicit_controls_and_stale_call_closes_picker() {
		let mut state = test_support::voice_demo_state();
		state.demo = false;
		state.gateway_connected = true;
		for guild in state.permissions.guilds.values_mut() {
			for role in guild.roles.as_mut().unwrap() {
				role.bits |= model::permissions::USE_SOUNDBOARD;
			}
		}
		state.permissions.clear_cache();
		let ctx = egui::Context::default();
		let mut view = SoundboardUi::default();
		let mut commands = Vec::new();
		view.open(&mut state, &mut commands);
		let Command::Soundboard(load) = commands.pop().unwrap() else {
			panic!("wrong command")
		};
		assert_eq!(load.action, Action::Load);
		frame(&ctx, &mut view, &mut state, &mut commands, vec![]);
		assert!(commands.is_empty());
		state.apply(client_core::Envelope {
			generation: state.generation,
			event: client_core::Event::Soundboard(client_core::soundboard::Event {
				scope: load.scope,
				request: load.request,
				result: Ok(client_core::soundboard::Outcome::Loaded(vec![
					model::soundboard::Sound {
						id: Id(99),
						name: "Synthetic duck".into(),
						volume: 1.0,
						emoji: Some("🦆".into()),
						emoji_id: None,
						guild: None,
						available: true,
					},
				])),
			}),
		});
		let _ = frame(&ctx, &mut view, &mut state, &mut commands, vec![]);
		let labels = frame(&ctx, &mut view, &mut state, &mut commands, vec![]);
		let button = labels
			.iter()
			.find(|(text, _)| text == "🦆 Synthetic duck")
			.unwrap()
			.1
			.center();
		assert!(commands.is_empty());
		for pressed in [true, false] {
			frame(
				&ctx,
				&mut view,
				&mut state,
				&mut commands,
				vec![
					egui::Event::PointerMoved(button),
					egui::Event::PointerButton {
						pos: button,
						button: egui::PointerButton::Primary,
						pressed,
						modifiers: Default::default(),
					},
				],
			);
		}
		assert!(matches!(
			commands.as_slice(),
			[Command::Soundboard(client_core::soundboard::Request {
				action: Action::Play(Id(99)),
				..
			})]
		));
		state.voice.active.as_mut().unwrap().request += 1;
		frame(&ctx, &mut view, &mut state, &mut commands, vec![]);
		assert!(view.open.is_none());
		assert!(state.soundboard.pending.is_none());
	}
	#[test]
	fn soundboard_demo_controls_never_request_network_or_playback() {
		let mut state = test_support::voice_demo_state();
		let ctx = egui::Context::default();
		let mut view = SoundboardUi::default();
		let mut commands = Vec::new();
		view.open(&mut state, &mut commands);
		let _ = frame(&ctx, &mut view, &mut state, &mut commands, vec![]);
		let labels = frame(&ctx, &mut view, &mut state, &mut commands, vec![]);
		assert!(
			labels
				.iter()
				.any(|(text, _)| text.starts_with("Offline preview"))
		);
		let button = labels
			.iter()
			.find(|(text, _)| text == "🦆 Quack")
			.unwrap()
			.1
			.center();
		for pressed in [true, false] {
			frame(
				&ctx,
				&mut view,
				&mut state,
				&mut commands,
				vec![
					egui::Event::PointerMoved(button),
					egui::Event::PointerButton {
						pos: button,
						button: egui::PointerButton::Primary,
						pressed,
						modifiers: Default::default(),
					},
				],
			);
		}
		assert!(commands.is_empty());
		assert!(state.soundboard.pending.is_none());
	}
}
