//! The in-call soundboard: Discord's default sounds, this server's sounds and other servers'.
//! Clicking a sound asks Discord to play it; nothing is mixed into this device's microphone.
use crate::{MessagingUi, design};
use client_core::{Command, State, soundboard::Load};
use egui::RichText;
use model::{Id, soundboard::Sound};

const LOADING: &str = "Loading sounds…";

/// One soundboard button exists, in the in-call card.
fn open_id() -> egui::Id {
	egui::Id::unique("serein-soundboard-open")
}

/// One titled collection in the picker.
#[derive(Debug)]
pub(crate) struct Section<'a> {
	pub title: String,
	/// Discord's default sounds have no server.
	pub guild: Option<Id>,
	pub sounds: Vec<&'a Sound>,
	/// Shown instead of sounds while loading, failed or empty.
	pub status: Option<&'static str>,
}

/// Default sounds first, then the call's server, then other servers in sidebar order.
/// A search hides collections without matches instead of showing them empty.
pub(crate) fn sections<'a>(state: &'a State, query: &str) -> Vec<Section<'a>> {
	let query = query.trim();
	let call_guild = state.voice.active.as_ref().and_then(|call| call.guild);
	let filter = |sounds: &'a [Sound]| -> Vec<&'a Sound> {
		sounds.iter().filter(|sound| sound.matches(query)).collect()
	};
	let mut sections = Vec::new();
	let defaults = filter(&state.soundboard.defaults);
	let default_status = match state.soundboard.defaults_load {
		Load::Failed(reason) => Some(reason),
		Load::Loaded => None,
		Load::Idle | Load::Loading => Some(LOADING),
	};
	if query.is_empty() || !defaults.is_empty() {
		sections.push(Section {
			title: "Default".into(),
			guild: None,
			status: default_status.filter(|_| defaults.is_empty()),
			sounds: defaults,
		});
	}
	let guild_name = |id: Id| {
		state
			.guilds
			.iter()
			.find(|guild| guild.id == id)
			.map_or_else(|| "Server".to_owned(), |guild| guild.name.clone())
	};
	if let Some(guild) = call_guild {
		let catalog = state.soundboard.guild(guild);
		let sounds = catalog.map(filter).unwrap_or_default();
		if query.is_empty() || !sounds.is_empty() {
			sections.push(Section {
				title: guild_name(guild),
				guild: Some(guild),
				status: match catalog {
					None => Some(LOADING),
					Some([]) => Some("This server has no sounds yet"),
					Some(_) => None,
				},
				sounds,
			});
		}
	}
	for guild in &state.guilds {
		if Some(guild.id) == call_guild {
			continue;
		}
		let sounds = state
			.soundboard
			.guild(guild.id)
			.map(filter)
			.unwrap_or_default();
		if !sounds.is_empty() {
			sections.push(Section {
				title: guild.name.clone(),
				guild: Some(guild.id),
				sounds,
				status: None,
			});
		}
	}
	sections
}

impl MessagingUi {
	/// The soundboard opened from the in-call card; loading starts when it opens.
	pub(crate) fn soundboard_popup(
		&mut self,
		trigger: &egui::Response,
		state: &mut State,
		commands: &mut Vec<Command>,
	) {
		let id = open_id();
		let mut open = trigger
			.ctx
			.data_mut(|data| *data.get_temp_mut_or_default::<bool>(id));
		let mut opened = false;
		if trigger.clicked() {
			open = !open;
			opened = open;
			if open {
				state.soundboard.error = None;
			}
		}
		if open {
			// Each frame only fills gaps left by a reconnect; an explicit open also retries failures.
			commands.extend(state.request_soundboard(opened));
		}
		egui::Popup::menu(trigger)
			.open_bool(&mut open)
			.style(|_: &mut egui::Style| {})
			.width(360.0)
			.close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
			.show(|ui| self.soundboard_content(ui, state, commands));
		trigger.ctx.data_mut(|data| data.insert_temp(id, open));
	}

	/// Offline previews start with the panel open; a click toggles it as usual.
	pub fn open_soundboard(ctx: &egui::Context) {
		ctx.data_mut(|data| data.insert_temp(open_id(), true));
	}

	/// Forget a stale open flag when the call ends or the trigger is disabled.
	pub(crate) fn close_soundboard(ctx: &egui::Context) {
		ctx.data_mut(|data| data.insert_temp(open_id(), false));
	}

	/// Whether the soundboard panel is open in this window.
	pub(crate) fn soundboard_open(&self, ctx: &egui::Context) -> bool {
		ctx.data(|data| data.get_temp::<bool>(open_id()).unwrap_or(false))
	}

	/// The panel body: search, catalog sections with their send and preview buttons, errors and
	/// the Soundboard volume slider. Clicked sounds become commands or local previews.
	pub(crate) fn soundboard_content(
		&mut self,
		ui: &mut egui::Ui,
		state: &mut State,
		commands: &mut Vec<Command>,
	) {
		let colors = design::palette(ui);
		ui.set_width(340.0);
		ui.spacing_mut().item_spacing.y = 6.0;
		ui.label(design::semibold(ui, "Soundboard", 15.0).color(colors.text_strong));
		design::input(
			ui,
			egui::TextEdit::singleline(&mut self.soundboard_query)
				.hint_text("Search sounds")
				.char_limit(32),
		);
		let mut chosen: Option<Sound> = None;
		let mut previewed: Option<Sound> = None;
		let shown = sections(state, &self.soundboard_query);
		let call_guild = state.voice.active.as_ref().and_then(|call| call.guild);
		egui::ScrollArea::vertical()
			.max_height(320.0)
			.auto_shrink([false, true])
			.show(ui, |ui| {
				if shown.is_empty() {
					ui.label(
						RichText::new("No sounds match your search")
							.size(13.0)
							.color(colors.muted),
					);
				}
				for section in &shown {
					ui.add_space(4.0);
					ui.label(
						RichText::new(section.title.to_uppercase())
							.size(11.0)
							.strong()
							.color(colors.muted),
					);
					if let Some(status) = section.status {
						ui.label(RichText::new(status).size(13.0).color(colors.muted));
						continue;
					}
					// Every sound in one collection shares the call and permission checks.
					let probe = Sound {
						id: Id(1),
						name: String::new(),
						volume: 1.0,
						emoji_id: None,
						emoji_name: None,
						guild: section.guild.or(call_guild),
						available: true,
					};
					let section_block = state.soundboard_block(&probe);
					let preview_block = state.soundboard_preview_block();
					ui.horizontal_wrapped(|ui| {
						ui.spacing_mut().item_spacing = egui::vec2(6.0, 6.0);
						for sound in &section.sounds {
							let block = if sound.available {
								section_block
							} else {
								Some("This sound is unavailable")
							};
							let playing = state.soundboard.is_sending(sound.id);
							let (play, preview) = self.sound_chip(
								ui,
								sound,
								block,
								preview_block,
								playing,
								state.demo,
							);
							if play {
								chosen = Some((*sound).clone());
							}
							if preview {
								previewed = Some((*sound).clone());
							}
						}
					});
				}
			});
		drop(shown);
		if let Some(sound) = previewed {
			state.preview_sound(&sound);
		}
		if let Some(sound) = chosen
			&& let Some(command) = state.play_sound(&sound)
		{
			commands.push(command);
		}
		if let Some(error) = state.soundboard.error {
			ui.label(RichText::new(error).size(12.0).color(colors.warning));
		}
		ui.separator();
		let label = ui.label(
			RichText::new("Soundboard volume")
				.size(13.0)
				.color(colors.muted),
		);
		design::slider(ui, &mut self.voice_gain.soundboard_percent, 0..=200, "%")
			.labelled_by(label.id)
			.on_hover_text("How loud sounds others play are on this device");
	}

	/// One sound: the pill plays it for everyone in the call; the speaker button previews it
	/// on this device only, as in Discord. Returns `(play, preview)` clicks.
	fn sound_chip(
		&mut self,
		ui: &mut egui::Ui,
		sound: &Sound,
		block: Option<&'static str>,
		preview_block: Option<&'static str>,
		playing: bool,
		demo: bool,
	) -> (bool, bool) {
		let colors = design::palette(ui);
		let image = match (sound.emoji_id, sound.emoji_name.as_deref()) {
			(Some(id), _) => self.avatars.custom_image(ui.ctx(), id, 18.0, demo),
			(None, Some(emoji)) => crate::emoji::image(ui.ctx(), emoji, 18.0),
			(None, None) => None,
		};
		// One measured rect per sound, so the wrapped row can break between sounds.
		const SPEAKER: f32 = 20.0;
		let text = RichText::new(&sound.name).size(13.0);
		let width = egui::WidgetText::from(text.clone())
			.into_galley(
				ui,
				Some(egui::TextWrapMode::Truncate),
				200.0,
				egui::TextStyle::Button,
			)
			.size()
			.x;
		let padding = ui.spacing().button_padding.x;
		let play_width = padding * 2.0
			+ width + if image.is_some() {
			18.0 + ui.spacing().icon_spacing
		} else {
			0.0
		};
		let (rect, _) = ui.allocate_exact_size(
			egui::vec2(play_width + SPEAKER + 6.0, 32.0),
			egui::Sense::hover(),
		);
		ui.painter().rect_filled(
			rect,
			8,
			if playing {
				colors.selected
			} else {
				colors.hover
			},
		);
		let button = match image {
			Some(image) => {
				egui::Button::image_and_text(image, text).image_tint_follows_text_color(false)
			}
			None => egui::Button::new(text),
		}
		.frame(false)
		.truncate();
		let play_rect = egui::Rect::from_min_size(rect.min, egui::vec2(play_width, rect.height()));
		let play = ui.put(play_rect, |ui: &mut egui::Ui| {
			ui.add_enabled(block.is_none(), button)
		});
		let play = match block {
			Some(reason) => play.on_disabled_hover_text(reason),
			None => play.on_hover_text(format!("Play {} for everyone", sound.name)),
		};
		let speaker_rect = egui::Rect::from_center_size(
			egui::pos2(play_rect.right() + SPEAKER / 2.0, rect.center().y),
			egui::Vec2::splat(SPEAKER),
		);
		let preview = ui
			.scope_builder(egui::UiBuilder::new().max_rect(speaker_rect), |ui| {
				ui.add_enabled_ui(preview_block.is_none(), |ui| {
					crate::icons::button(
						ui,
						crate::icons::Icon::Speaker,
						SPEAKER,
						&format!("Preview {} (only you hear it)", sound.name),
					)
				})
				.inner
			})
			.inner;
		let preview = match preview_block {
			Some(reason) => preview.on_disabled_hover_text(reason),
			None => preview,
		};
		(play.clicked(), preview.clicked())
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use client_core::{
		auth::AuthState,
		soundboard::{Command as SoundCommand, Event},
		voice::{Call, Phase},
	};
	use egui::{Pos2, Rect};

	/// A synthetic sound named `name`, optionally owned by `guild`.
	fn sound(id: u64, name: &str, guild: Option<u64>) -> Sound {
		Sound {
			id: Id(id),
			name: name.into(),
			volume: 1.0,
			emoji_id: None,
			emoji_name: None,
			guild: guild.map(Id),
			available: true,
		}
	}
	/// An account in a connected call of server 10 with synthetic default and server catalogs.
	fn state() -> State {
		let guild = |id: u64, name: &str| model::Guild {
			default_message_notifications: None,
			id: Id(id),
			name: name.into(),
			icon: None,
			emojis: None,
			stickers: None,
		};
		let mut state = State {
			auth: AuthState::Authenticated,
			gateway_connected: true,
			user: Some(model::User {
				id: Id(1),
				name: "Owner".into(),
				avatar: None,
				webhook: false,
				kind: Default::default(),
				discriminator: 0,
				primary_guild: None,
				cosmetics: None,
			}),
			guilds: vec![
				guild(11, "Elsewhere"),
				guild(10, "Home"),
				guild(12, "Quiet"),
			],
			channels: vec![model::Channel {
				id: Id(20),
				guild: Some(Id(10)),
				kind: 2,
				name: "Voice".into(),
				last_message: None,
				parent_id: None,
				position: 0,
				recipients: vec![],
				icon: None,
				member_list_id: None,
				tags: None,
				message_count: None,
			}],
			..Default::default()
		};
		let snapshot = test_support::permission_snapshot(&state);
		state.permissions.replace(snapshot).unwrap();
		state.voice.active = Some(Call {
			channel: Id(20),
			guild: Some(Id(10)),
			connected_at: None,
			channel_started_at: None,
			server_muted: false,
			server_deafened: false,
			request: 1,
			phase: Phase::Connected,
			muted: false,
			deafened: false,
			participants: Vec::new(),
			camera: false,
			watching: None,
			error: None,
			reconnecting: None,
		});
		state
	}

	/// Section order, search and the loading and empty states.
	#[test]
	fn collections_are_ordered_searched_and_report_loading() {
		let mut state = state();
		let titles = |state: &State, query: &str| {
			sections(state, query)
				.into_iter()
				.map(|section| (section.title, section.status, section.sounds.len()))
				.collect::<Vec<_>>()
		};
		assert_eq!(
			titles(&state, ""),
			[
				("Default".into(), Some(LOADING), 0),
				("Home".into(), Some(LOADING), 0)
			]
		);
		state.apply_soundboard(Event::Defaults(Ok(vec![sound(1, "Quack", None)])));
		for (guild, sounds) in [
			(10, vec![sound(5, "Airhorn", Some(10))]),
			(
				11,
				vec![sound(6, "Air raid", Some(11)), sound(7, "Bell", Some(11))],
			),
			(12, vec![]),
		] {
			state.apply_soundboard(Event::Guild {
				guild: Id(guild),
				sounds,
			});
		}
		assert_eq!(
			titles(&state, ""),
			[
				("Default".into(), None, 1),
				("Home".into(), None, 1),
				("Elsewhere".into(), None, 2),
			]
		);
		assert_eq!(
			titles(&state, " AIR "),
			[("Home".into(), None, 1), ("Elsewhere".into(), None, 1)]
		);
		assert!(titles(&state, "nothing").is_empty());
	}

	/// A sound click sends a request and the sender hears it once; the speaker button only
	/// previews locally; a locked sound sends nothing.
	#[test]
	fn clicking_plays_for_everyone_and_the_speaker_previews_only_locally() {
		let mut state = state();
		state.apply_soundboard(Event::Defaults(Ok(vec![sound(1, "Quack", None)])));
		state.apply_soundboard(Event::Guild {
			guild: Id(11),
			sounds: vec![sound(6, "Foreign", Some(11))],
		});
		state.apply_soundboard(Event::Guild {
			guild: Id(10),
			sounds: vec![],
		});
		let ctx = egui::Context::default();
		let mut view = MessagingUi::default();
		let run = |view: &mut MessagingUi, state: &mut State, events: Vec<egui::Event>| {
			let mut commands = Vec::new();
			let mut labels = Vec::new();
			let output = ctx.run_ui(
				egui::RawInput {
					screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(400.0, 600.0))),
					events,
					..Default::default()
				},
				|ui| {
					egui::CentralPanel::default().show(ui, |ui| {
						view.soundboard_content(ui, state, &mut commands);
					});
				},
			);
			/// Gather every text shape with its rectangle, descending into nested shapes.
			fn collect(shape: &egui::Shape, labels: &mut Vec<(String, Rect)>) {
				match shape {
					egui::Shape::Text(t) => labels.push((
						t.galley.job.text.clone(),
						Rect::from_min_size(t.pos, t.galley.size()),
					)),
					egui::Shape::Vec(shapes) => shapes.iter().for_each(|s| collect(s, labels)),
					_ => {}
				}
			}
			for shape in &output.shapes {
				collect(&shape.shape, &mut labels);
			}
			output.drop_without_applying_deltas();
			(commands, labels)
		};
		let click = |rect: Rect| {
			let position = rect.center();
			[true, false].map(|pressed| {
				vec![
					egui::Event::PointerMoved(position),
					egui::Event::PointerButton {
						pos: position,
						button: egui::PointerButton::Primary,
						pressed,
						modifiers: Default::default(),
					},
				]
			})
		};
		let (_, labels) = run(&mut view, &mut state, Vec::new());
		let at = |name: &str| labels.iter().find(|(text, _)| text == name).unwrap().1;
		let (foreign, quack) = (at("Foreign"), at("Quack"));
		// Another server's sound without Nitro is disabled and sends nothing.
		let mut sent = Vec::new();
		for events in click(foreign) {
			sent.extend(run(&mut view, &mut state, events).0);
		}
		assert!(sent.is_empty());
		for events in click(quack) {
			sent.extend(run(&mut view, &mut state, events).0);
		}
		let [
			Command::Soundboard(SoundCommand::Play {
				sound: Id(1),
				channel: Id(20),
				source_guild: None,
				request,
			}),
		] = sent.as_slice()
		else {
			panic!("one default sound was sent");
		};
		state.apply_soundboard(Event::Played {
			request: *request,
			result: Ok(()),
		});
		// The accepted sound plays once for its sender.
		assert!(matches!(
			state.take_sound_effects().as_slice(),
			[client_core::soundboard::Playback {
				effect: model::soundboard::SoundEffect { sound: Id(1), .. },
				source: client_core::soundboard::Source::Own,
			}]
		));
		// The speaker button after a locked sound previews it locally and sends nothing.
		let speaker = Rect::from_center_size(
			foreign.right_center() + egui::vec2(14.0, 0.0),
			egui::Vec2::splat(1.0),
		);
		let mut sent = Vec::new();
		for events in click(speaker) {
			sent.extend(run(&mut view, &mut state, events).0);
		}
		assert!(sent.is_empty());
		assert!(matches!(
			state.take_sound_effects().as_slice(),
			[client_core::soundboard::Playback {
				effect: model::soundboard::SoundEffect {
					sound: Id(6),
					channel: Id(20),
					..
				},
				source: client_core::soundboard::Source::Preview,
			}]
		));
	}
}
