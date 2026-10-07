//! Native server soundboard catalog and bounded upload/edit/delete flows.
use crate::{avatars::Avatars, design, icons};
use client_core::{Command, State};
use egui::RichText;
use model::{Id, Patch, server_admin::Action};

/// Name, duration in milliseconds, waveform peaks (one per 10 ms) and, when the file can be
/// uploaded untouched, its content type and bytes.
pub(super) type PreparedSound = (String, u32, Vec<u8>, Option<(String, Vec<u8>)>);

/// Discord's limit for one sound, and the shortest selection offered.
const MAX_MS: f32 = 5200.0;
const MIN_MS: f32 = 200.0;
/// Longer files zoom to this span while a trim handle is held still for a second.
const ZOOM_FROM_MS: f32 = 20_000.0;
const ZOOM_SPAN_MS: f32 = 15_000.0;
/// Pointer jitter, in points, that still counts as holding a handle still.
const STILL_PX: f32 = 4.0;
/// Five minutes of 10 ms peaks.
const MAX_PEAKS: usize = 30_000;

#[derive(Clone, Copy, PartialEq)]
enum Grab {
	Start,
	End,
	Body,
}
struct Held {
	grab: Grab,
	/// Pointer position last applied to the selection.
	x: f32,
	/// Where and when the pointer last came to rest; small jitters do not restart it.
	rest: f32,
	since: f64,
	/// Distance from the selection start to the grabbed point when moving the whole selection.
	offset: f32,
}
enum Trim {
	Idle,
	Requested,
	Encoding,
	Ready(Vec<u8>),
}

struct Upload {
	name: String,
	emoji: String,
	volume: u8,
	duration: f32,
	peaks: Vec<u8>,
	original: Option<(String, Vec<u8>)>,
	/// Selected part in milliseconds.
	start: f32,
	end: f32,
	/// Visible part of the waveform, eased toward `target`.
	view: (f32, f32),
	target: (f32, f32),
	held: Option<Held>,
	trim: Trim,
	/// A pending request to play (`Some`) or stop (`None`) the selection locally.
	preview: Option<Option<(u32, u32, u8)>>,
	playing: Option<f64>,
}
impl Upload {
	fn stop_preview(&mut self) {
		if self.playing.take().is_some() {
			self.preview = Some(None);
		}
	}
	/// Move one edge or the whole selection to `at`, keeping it within 0.2 to 5.2 seconds.
	fn drag(&mut self, grab: Grab, at: f32, offset: f32) {
		match grab {
			Grab::Start => {
				self.start = at.clamp(0.0, (self.duration - MIN_MS).max(0.0));
				self.end = self
					.end
					.clamp(self.start + MIN_MS, self.start + MAX_MS)
					.min(self.duration);
			}
			Grab::End => {
				self.end = at.clamp(MIN_MS.min(self.duration), self.duration);
				self.start = self
					.start
					.clamp(self.end - MAX_MS, self.end - MIN_MS)
					.max(0.0);
			}
			Grab::Body => {
				let length = self.end - self.start;
				self.start = (at - offset).clamp(0.0, (self.duration - length).max(0.0));
				self.end = self.start + length;
			}
		}
	}
	/// The whole file is selected, so an uploadable original needs no re-encoding.
	fn whole(&self) -> bool {
		self.start <= 0.5 && self.end >= self.duration - 0.5
	}
}

enum Dialog {
	Edit {
		id: Id,
		name: String,
		emoji: String,
		/// The Unicode emoji the sound had when the dialog opened; empty for none or custom.
		original: String,
		custom: bool,
		volume: u8,
	},
	Delete {
		id: Id,
		name: String,
	},
}

#[derive(Default)]
pub(super) struct SoundsUi {
	pub request: u64,
	choosing: bool,
	request_started: bool,
	upload: Option<Upload>,
	submitted_upload: bool,
	dialog: Option<Dialog>,
	dialog_submitted: bool,
	error: Option<&'static str>,
	closed: bool,
}

impl SoundsUi {
	pub fn has_changes(&self) -> bool {
		self.choosing || self.upload.is_some() || self.dialog_submitted
	}

	pub fn load(&mut self, state: &mut State, guild: Id) -> Option<Command> {
		if state.server_admin.pending
			|| (state.server_admin.guild == Some(guild)
				&& (state.server_admin.error.is_some() || state.server_admin.sounds.is_some()))
		{
			return None;
		}
		state.request_server_admin(guild, Action::LoadSounds)
	}

	fn choose(&mut self) {
		if self.upload.is_none() && !self.choosing {
			self.choosing = true;
			self.error = None;
		}
	}

	pub fn take_request(&mut self) -> bool {
		if !self.choosing || self.request_started {
			return false;
		}
		self.request_started = true;
		true
	}

	pub fn accept(&mut self, result: Result<Option<PreparedSound>, &'static str>) {
		if !self.choosing || !self.request_started {
			return;
		}
		self.choosing = false;
		self.request_started = false;
		match result {
			Ok(Some((name, duration_ms, peaks, original)))
				if duration_ms > 0
					&& !peaks.is_empty()
					&& peaks.len() <= MAX_PEAKS
					&& original.as_ref().is_none_or(|(_, file)| {
						file.len() <= model::server_admin::MAX_SOUND_FILE_BYTES
					}) =>
			{
				let duration = duration_ms as f32;
				self.upload = Some(Upload {
					name,
					emoji: String::new(),
					volume: 100,
					duration,
					peaks,
					original,
					start: 0.0,
					end: duration.min(MAX_MS),
					view: (0.0, duration),
					target: (0.0, duration),
					held: None,
					trim: Trim::Idle,
					preview: None,
					playing: None,
				});
			}
			Ok(Some(_)) => self.error = Some("This sound could not be prepared"),
			Ok(None) => {}
			Err(error) => self.error = Some(error),
		}
	}

	/// The selection to encode, once per upload click that needs a trim.
	pub fn take_trim(&mut self) -> Option<(u32, u32)> {
		let upload = self.upload.as_mut()?;
		if !matches!(upload.trim, Trim::Requested) {
			return None;
		}
		upload.trim = Trim::Encoding;
		Some((upload.start as u32, upload.end as u32))
	}

	pub fn accept_trim(&mut self, result: Result<Vec<u8>, &'static str>) {
		let Some(upload) = &mut self.upload else {
			return;
		};
		if !matches!(upload.trim, Trim::Encoding) {
			return;
		}
		match result {
			Ok(file)
				if !file.is_empty() && file.len() <= model::server_admin::MAX_SOUND_FILE_BYTES =>
			{
				upload.trim = Trim::Ready(file)
			}
			Ok(_) => {
				upload.trim = Trim::Idle;
				self.error = Some("Sounds can be at most 512 KB");
			}
			Err(error) => {
				upload.trim = Trim::Idle;
				self.error = Some(error);
			}
		}
	}

	/// A request to play (`Some`) or stop (`None`) the selection on this device.
	pub fn take_preview(&mut self) -> Option<Option<(u32, u32, u8)>> {
		self.upload.as_mut()?.preview.take()
	}

	/// True once after the review closed, so the desktop can release the decoded audio.
	pub fn take_closed(&mut self) -> bool {
		std::mem::take(&mut self.closed)
	}

	/// Open the review with an already prepared sound, without a file picker.
	#[cfg(feature = "demo")]
	pub fn preview_upload(&mut self, prepared: PreparedSound) {
		self.choosing = true;
		self.request_started = true;
		self.accept(Ok(Some(prepared)));
	}

	fn upload_dialog(
		&mut self,
		ctx: &egui::Context,
		state: &mut State,
		guild: Id,
		commands: &mut Vec<Command>,
	) {
		let Some(upload) = &mut self.upload else {
			return;
		};
		// A finished trim becomes the upload request.
		if matches!(upload.trim, Trim::Ready(_))
			&& !state.server_admin.pending
			&& let Trim::Ready(file) = std::mem::replace(&mut upload.trim, Trim::Idle)
		{
			match state.request_server_admin(
				guild,
				Action::CreateSound {
					name: upload.name.trim().to_owned(),
					emoji: upload.emoji.trim().to_owned(),
					volume: upload.volume.min(100),
					content_type: "audio/ogg".into(),
					file,
				},
			) {
				Some(command) => {
					self.submitted_upload = true;
					commands.push(command);
				}
				None => self.error = Some("This sound can no longer be uploaded here"),
			}
		}
		let encoding = !matches!(upload.trim, Trim::Idle);
		let busy = state.server_admin.pending || encoding;
		let valid = valid_fields(&upload.name, &upload.emoji);
		let error = state.server_admin.error.or(self.error);
		let mut submit = false;
		let mut cancel = false;
		let response = crate::dialog::Dialog::new(
			"server-sound-upload",
			crate::i18n::translate("server-sounds-upload-title"),
		)
		.width(480.0)
		.show(ctx, |dialog_ui| {
			dialog_ui.content(|ui| {
				crate::dialog::label(ui, "server-sounds-preview");
				waveform(ui, upload, !busy);
				crate::dialog::hint(
					ui,
					&crate::i18n::translate(if upload.duration > MAX_MS {
						"server-sounds-trim-hint"
					} else {
						"server-sounds-hint"
					}),
				);
				ui.add_space(12.0);
				fields(ui, &mut upload.name, &mut upload.emoji, &mut upload.volume);
				if !valid {
					design::notice(ui, design::Level::Error, "server-sounds-invalid");
				}
				if let Some(error) = error {
					design::notice(ui, design::Level::Error, error);
				}
				if busy {
					ui.horizontal(|ui| {
						ui.spinner();
						ui.weak(crate::i18n::translate(if encoding {
							"server-sounds-preparing"
						} else {
							"server-sounds-saving"
						}));
					});
				}
			});
			dialog_ui.footer(|ui| {
				submit = ui
					.add_enabled_ui(valid && !busy, |ui| {
						crate::dialog::action(
							ui,
							"server-sounds-upload",
							crate::dialog::Action::Primary,
						)
					})
					.inner
					.clicked();
				cancel = crate::dialog::action(
					ui,
					"server-sounds-never-mind",
					crate::dialog::Action::Neutral,
				)
				.clicked();
			});
		});
		if submit {
			upload.stop_preview();
			self.error = None;
			match &upload.original {
				Some((content_type, file)) if upload.whole() => {
					if let Some(command) = state.request_server_admin(
						guild,
						Action::CreateSound {
							name: upload.name.trim().to_owned(),
							emoji: upload.emoji.trim().to_owned(),
							volume: upload.volume.min(100),
							content_type: content_type.clone(),
							file: file.clone(),
						},
					) {
						self.submitted_upload = true;
						commands.push(command);
					}
				}
				_ => upload.trim = Trim::Requested,
			}
		}
		if (response.close || cancel) && !self.submitted_upload {
			self.upload = None;
			self.closed = true;
		}
	}

	pub fn show(
		&mut self,
		ui: &mut egui::Ui,
		state: &mut State,
		guild: Id,
		avatars: &mut Avatars,
		commands: &mut Vec<Command>,
	) {
		if self.submitted_upload && !state.server_admin.pending {
			self.submitted_upload = false;
			if state.server_admin.error.is_none() {
				self.upload = None;
				self.closed = true;
			}
		}
		if self.dialog_submitted && !state.server_admin.pending {
			self.dialog_submitted = false;
			if state.server_admin.error.is_none() {
				self.dialog = None;
			}
		}
		let can_create = state.can_create_guild_sound(guild);
		if !can_create {
			self.closed |= self.upload.take().is_some();
			self.choosing = false;
			self.request_started = false;
		}
		let full = state.server_admin.sounds.as_ref().is_some_and(|catalog| {
			catalog
				.limit
				.is_some_and(|limit| catalog.items.len() >= limit)
		});

		design::page_header(
			ui,
			"server-sounds-title",
			Some("server-sounds-description"),
			|ui| {
				if can_create
					&& ui
						.add_enabled_ui(
							!self.choosing
								&& self.upload.is_none() && !state.server_admin.pending
								&& !full,
							|ui| {
								design::button(
									ui,
									"server-sounds-upload-sound",
									design::ButtonKind::Primary,
								)
							},
						)
						.inner
						.clicked()
				{
					self.choose();
				}
			},
		);
		if let Some(error) = state.server_admin.error.or(self.error) {
			design::notice(ui, design::Level::Error, error);
			if ui
				.add_enabled(
					!state.server_admin.pending,
					egui::Button::new(crate::i18n::translate("server-sounds-reload")),
				)
				.clicked() && let Some(command) =
				state.request_server_admin(guild, Action::LoadSounds)
			{
				commands.push(command);
				self.error = None;
			}
		}
		if state.server_admin.pending {
			ui.horizontal(|ui| {
				ui.spinner();
				ui.weak(crate::i18n::translate(if state.server_admin.saving {
					"server-sounds-saving"
				} else {
					"server-sounds-loading"
				}));
			});
		}
		if can_create {
			design::hint(ui, "server-sounds-hint");
			ui.add_space(12.0);
		}
		if self.choosing {
			ui.weak(crate::i18n::translate("server-sounds-preparing"));
		}
		self.upload_dialog(ui.ctx(), state, guild, commands);

		ui.add_space(24.0);
		let Some(catalog) = state.server_admin.sounds.as_ref() else {
			return;
		};
		let count = catalog.items.len();
		let usage = catalog.limit.map_or_else(
			|| crate::i18n::translate_args("server-sounds-count", &[("count", &count.to_string())]),
			|limit| {
				crate::i18n::translate_args(
					"server-sounds-slots",
					&[
						("count", &limit.saturating_sub(count).to_string()),
						("limit", &limit.to_string()),
					],
				)
			},
		);
		design::section(ui, "server-sounds-section", Some(&usage));
		if catalog.items.is_empty() {
			design::card(ui, |ui| {
				design::empty_state(
					ui,
					icons::Icon::Soundboard,
					"server-sounds-empty",
					if can_create {
						"server-sounds-empty-detail"
					} else {
						""
					},
				);
			});
		} else {
			design::card(ui, |ui| {
				let width = ui.available_width();
				let name_width = ((width - 116.0) * 0.5).max(60.0);
				let by_width = (width - name_width - 116.0).max(40.0);
				ui.horizontal(|ui| {
					heading(ui, "server-sounds-column-emoji", 52.0);
					heading(ui, "server-sounds-column-name", name_width);
					heading(ui, "server-sounds-column-uploaded-by", by_width);
				});
				ui.separator();
				for row in &catalog.items {
					ui.push_id(row.sound.id, |ui| {
						ui.horizontal(|ui| {
							let (cell, _) = ui
								.allocate_exact_size(egui::vec2(52.0, 44.0), egui::Sense::hover());
							let glyph = egui::Rect::from_center_size(
								egui::pos2(cell.left() + 14.0, cell.center().y),
								egui::Vec2::splat(24.0),
							);
							if row.sound.emoji_id.is_some() || row.sound.emoji_name.is_some() {
								crate::forum::paint_emoji(
									ui,
									(avatars, state.demo),
									(row.sound.emoji_id, row.sound.emoji_name.as_deref()),
									glyph,
								);
							} else {
								icons::paint(
									ui.painter(),
									icons::Icon::Soundboard,
									glyph.shrink(2.0),
									design::palette(ui).muted,
								);
							}
							ui.allocate_ui_with_layout(
								egui::vec2(name_width, 44.0),
								egui::Layout::left_to_right(egui::Align::Center),
								|ui| {
									ui.set_width(name_width);
									ui.add(
										egui::Label::new(design::medium(ui, &row.sound.name, 14.0))
											.truncate(),
									);
									if !row.sound.available {
										ui.weak(crate::i18n::translate(
											"server-sounds-unavailable",
										));
									}
								},
							);
							ui.allocate_ui_with_layout(
								egui::vec2(by_width, 44.0),
								egui::Layout::left_to_right(egui::Align::Center),
								|ui| {
									ui.set_max_width(by_width);
									if let Some(user) = &row.uploader {
										avatars.show(ui, user, 24.0, state.demo);
										ui.add(egui::Label::new(&user.name).truncate());
									} else {
										ui.weak(crate::i18n::translate("server-sounds-unknown"));
									}
								},
							);
							if state.can_edit_guild_sound(guild, row.sound.id) {
								let button = icons::button(
									ui,
									icons::Icon::More,
									22.0,
									"server-sounds-actions",
								);
								egui::Popup::menu(&button).show(|ui| {
									if ui
										.button(crate::i18n::translate("server-sounds-edit"))
										.clicked()
									{
										let custom = row.sound.emoji_id.is_some();
										let emoji = row
											.sound
											.emoji_name
											.clone()
											.filter(|_| !custom)
											.unwrap_or_default();
										self.dialog = Some(Dialog::Edit {
											id: row.sound.id,
											name: row.sound.name.clone(),
											original: emoji.clone(),
											emoji,
											custom,
											volume: (row.sound.volume * 100.0).round() as u8,
										});
										ui.close();
									}
									if ui
										.button(
											RichText::new(crate::i18n::translate(
												"server-sounds-delete",
											))
											.color(design::palette(ui).danger),
										)
										.clicked()
									{
										self.dialog = Some(Dialog::Delete {
											id: row.sound.id,
											name: row.sound.name.clone(),
										});
										ui.close();
									}
								});
							}
						});
					});
				}
			});
		}
		self.dialog(ui.ctx(), state, guild, commands);
	}

	fn dialog(
		&mut self,
		ctx: &egui::Context,
		state: &mut State,
		guild: Id,
		commands: &mut Vec<Command>,
	) {
		let Some(dialog) = &mut self.dialog else {
			return;
		};
		let (title, subtitle, danger) = match dialog {
			Dialog::Edit { .. } => (
				crate::i18n::translate("server-sounds-edit-title"),
				crate::i18n::translate("server-sounds-edit-subtitle"),
				false,
			),
			Dialog::Delete { name, .. } => (
				crate::i18n::translate("server-sounds-delete-title"),
				crate::i18n::translate_args("server-sounds-delete-subtitle", &[("name", name)]),
				true,
			),
		};
		let mut action = None;
		let mut cancel = false;
		let mut builder = crate::dialog::Dialog::new("server-sound-dialog", title)
			.subtitle(subtitle)
			.width(440.0);
		if danger {
			builder = builder.danger();
		}
		let response = builder.show(ctx, |dialog_ui| {
			dialog_ui.content(|ui| match dialog {
				Dialog::Edit {
					name,
					emoji,
					custom,
					volume,
					..
				} => {
					fields(ui, name, emoji, volume);
					if *custom {
						crate::dialog::hint(
							ui,
							&crate::i18n::translate("server-sounds-custom-emoji-kept"),
						);
					}
				}
				Dialog::Delete { .. } => {}
			});
			dialog_ui.footer(|ui| {
				match dialog {
					Dialog::Edit {
						id,
						name,
						emoji,
						original,
						custom,
						volume,
					} => {
						if ui
							.add_enabled_ui(
								!state.server_admin.pending
									&& state.can_edit_guild_sound(guild, *id)
									&& valid_fields(name, emoji),
								|ui| {
									crate::dialog::action(
										ui,
										"server-sounds-save",
										crate::dialog::Action::Primary,
									)
								},
							)
							.inner
							.clicked()
						{
							action = Some(Action::EditSound {
								id: *id,
								name: name.trim().to_owned(),
								emoji: emoji_patch(emoji.trim(), original, *custom),
								volume: (*volume).min(100),
							});
						}
					}
					Dialog::Delete { id, .. } => {
						if ui
							.add_enabled_ui(
								!state.server_admin.pending
									&& state.can_edit_guild_sound(guild, *id),
								|ui| {
									crate::dialog::action(
										ui,
										"server-sounds-delete",
										crate::dialog::Action::Danger,
									)
								},
							)
							.inner
							.clicked()
						{
							action = Some(Action::DeleteSound { id: *id });
						}
					}
				}
				cancel = crate::dialog::action(
					ui,
					"server-sounds-cancel",
					crate::dialog::Action::Neutral,
				)
				.clicked();
			});
		});
		if let Some(action) = action.and_then(|action| state.request_server_admin(guild, action)) {
			self.dialog_submitted = true;
			commands.push(action);
		}
		if (response.close || cancel) && !self.dialog_submitted {
			self.dialog = None;
		}
	}
}

/// The trim control: a play button, the selection length, and a waveform whose two handles
/// and selected span can be dragged. Holding a handle still on a long file zooms in.
fn waveform(ui: &mut egui::Ui, upload: &mut Upload, enabled: bool) {
	let colors = design::palette(ui);
	let (rect, _) =
		ui.allocate_exact_size(egui::vec2(ui.available_width(), 96.0), egui::Sense::hover());
	ui.painter().rect_filled(rect, 8, colors.raised);
	let now = ui.input(|input| input.time);
	let id = ui.scope_id().with("server-sound-trim");

	// Ease the visible span toward its target.
	let step = 1.0 - (-ui.input(|input| input.stable_dt).min(0.1) * 10.0).exp();
	let moving = (upload.target.0 - upload.view.0).abs() + (upload.target.1 - upload.view.1).abs();
	if moving > 1.0 {
		upload.view.0 += (upload.target.0 - upload.view.0) * step;
		upload.view.1 += (upload.target.1 - upload.view.1) * step;
		ui.ctx().request_repaint();
	} else {
		upload.view = upload.target;
	}
	let (from, to) = upload.view;
	let wave = egui::Rect::from_min_max(
		egui::pos2(rect.left() + 72.0, rect.top() + 12.0),
		egui::pos2(rect.right() - 16.0, rect.bottom() - 12.0),
	);
	let span = (to - from).max(1.0);
	let x_of = |ms: f32| wave.left() + (ms - from) / span * wave.width();
	let ms_of = |x: f32| from + (x - wave.left()) / wave.width() * span;

	let button = egui::Rect::from_center_size(
		egui::pos2(rect.left() + 36.0, rect.center().y - 8.0),
		egui::Vec2::splat(32.0),
	);
	let play = ui.interact(
		button,
		id.with("play"),
		if enabled {
			egui::Sense::click()
		} else {
			egui::Sense::hover()
		},
	);
	let label = crate::i18n::translate(if upload.playing.is_some() {
		"server-sounds-stop-preview"
	} else {
		"server-sounds-play-preview"
	});
	play.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, enabled, &label));
	ui.painter().circle_filled(
		button.center(),
		16.0,
		if play.hovered() || play.has_focus() {
			colors.hover
		} else {
			colors.border
		},
	);
	let center = button.center();
	if upload.playing.is_some() {
		ui.painter().rect_filled(
			egui::Rect::from_center_size(center, egui::Vec2::splat(10.0)),
			2,
			colors.text_strong,
		);
	} else {
		ui.painter().add(egui::Shape::convex_polygon(
			vec![
				center + egui::vec2(-4.0, -6.0),
				center + egui::vec2(-4.0, 6.0),
				center + egui::vec2(7.0, 0.0),
			],
			colors.text_strong,
			egui::Stroke::NONE,
		));
	}
	if play.clicked() {
		if upload.playing.is_some() {
			upload.stop_preview();
		} else {
			upload.playing = Some(now);
			upload.preview = Some(Some((
				upload.start as u32,
				upload.end as u32,
				upload.volume.min(100),
			)));
		}
	}
	play.on_hover_text(label);
	ui.painter().text(
		egui::pos2(button.center().x, button.bottom() + 12.0),
		egui::Align2::CENTER_CENTER,
		format!("{:.2}s", (upload.end - upload.start) / 1000.0),
		egui::FontId::proportional(11.0),
		if upload.end - upload.start > MAX_MS + 0.5 {
			colors.warning
		} else {
			colors.muted
		},
	);

	// One 2 px bar every 3 px, lit inside the selection.
	let painter = ui
		.painter()
		.with_clip_rect(wave.expand2(egui::vec2(8.0, 6.0)));
	let mut x = wave.left();
	while x < wave.right() {
		let (a, b) = (ms_of(x), ms_of(x + 3.0));
		if b > 0.0 && a < upload.duration {
			let first = (a.max(0.0) / 10.0) as usize;
			let last = ((b.min(upload.duration) / 10.0).ceil() as usize).max(first + 1);
			let peak = upload
				.peaks
				.get(first..last.min(upload.peaks.len()))
				.and_then(|peaks| peaks.iter().max())
				.copied()
				.unwrap_or(0);
			let height = (f32::from(peak) / 255.0 * wave.height()).max(2.0);
			let middle = (a + b) * 0.5;
			painter.rect_filled(
				egui::Rect::from_center_size(
					egui::pos2(x + 1.0, wave.center().y),
					egui::vec2(2.0, height),
				),
				1,
				if (upload.start..=upload.end).contains(&middle) {
					colors.text_strong
				} else {
					colors.muted.gamma_multiply(0.55)
				},
			);
		}
		x += 3.0;
	}

	let (start_x, end_x) = (x_of(upload.start), x_of(upload.end));
	let handle = |x: f32| {
		egui::Rect::from_center_size(
			egui::pos2(x, wave.center().y),
			egui::vec2(16.0, wave.height() + 12.0),
		)
	};
	let sense = if enabled {
		egui::Sense::drag()
	} else {
		egui::Sense::hover()
	};
	// Later widgets win overlapping hits: the handles sit above the selected span.
	let grips = [
		(
			Grab::Body,
			ui.interact(
				egui::Rect::from_min_max(
					egui::pos2(start_x, wave.top()),
					egui::pos2(end_x.max(start_x), wave.bottom()),
				),
				id.with("body"),
				sense,
			),
		),
		(Grab::End, ui.interact(handle(end_x), id.with("end"), sense)),
		(
			Grab::Start,
			ui.interact(handle(start_x), id.with("start"), sense),
		),
	];
	for (grab, response) in grips {
		if response.hovered() || response.dragged() {
			ui.ctx().set_cursor_icon(if grab == Grab::Body {
				egui::CursorIcon::Grab
			} else {
				egui::CursorIcon::ResizeHorizontal
			});
		}
		if response.drag_started()
			&& let Some(pos) = response.interact_pointer_pos()
		{
			upload.stop_preview();
			upload.held = Some(Held {
				grab,
				x: pos.x,
				rest: pos.x,
				since: now,
				offset: ms_of(pos.x) - upload.start,
			});
		}
		if response.dragged()
			&& let Some(pos) = response.interact_pointer_pos()
			&& let Some(held) = upload.held.as_mut().filter(|held| held.grab == grab)
		{
			if (pos.x - held.rest).abs() > STILL_PX {
				held.rest = pos.x;
				held.since = now;
			}
			let still = now - held.since >= 1.0;
			if (pos.x - held.x).abs() > 0.5 {
				held.x = pos.x;
				let offset = held.offset;
				upload.drag(grab, ms_of(pos.x), offset);
			}
			if still
				&& grab != Grab::Body
				&& upload.duration > ZOOM_FROM_MS
				&& upload.target == (0.0, upload.duration)
			{
				// Zoom around the held handle so it stays under the pointer.
				let fraction = ((pos.x - wave.left()) / wave.width()).clamp(0.0, 1.0);
				let anchor = ms_of(pos.x);
				upload.target = (
					anchor - fraction * ZOOM_SPAN_MS,
					anchor + (1.0 - fraction) * ZOOM_SPAN_MS,
				);
			}
			// Keep frames coming so a still pointer is noticed.
			ui.ctx().request_repaint();
		}
		if response.drag_stopped() {
			upload.held = None;
			upload.target = (0.0, upload.duration);
		}
	}

	for x in [x_of(upload.start), x_of(upload.end)] {
		painter.rect_filled(
			egui::Rect::from_center_size(
				egui::pos2(x, wave.center().y),
				egui::vec2(2.0, wave.height() + 8.0),
			),
			1,
			colors.text_strong,
		);
		painter.rect_filled(
			egui::Rect::from_center_size(egui::pos2(x, wave.center().y), egui::vec2(6.0, 20.0)),
			3,
			colors.text_strong,
		);
	}
	if let Some(started) = upload.playing {
		let elapsed = ((now - started) * 1000.0) as f32;
		if elapsed >= upload.end - upload.start {
			upload.playing = None;
		} else {
			painter.rect_filled(
				egui::Rect::from_center_size(
					egui::pos2(x_of(upload.start + elapsed), wave.center().y),
					egui::vec2(2.0, wave.height() + 8.0),
				),
				1,
				colors.accent,
			);
			ui.ctx().request_repaint();
		}
	}
}
/// Name, related emoji and volume, shared by the upload review and the edit dialog.
fn fields(ui: &mut egui::Ui, name: &mut String, emoji: &mut String, volume: &mut u8) {
	let label = crate::dialog::label(ui, "server-sounds-name");
	crate::dialog::input(ui, egui::TextEdit::singleline(name).char_limit(32)).labelled_by(label.id);
	ui.add_space(12.0);
	let label = crate::dialog::label(ui, "server-sounds-emoji");
	crate::dialog::input(ui, egui::TextEdit::singleline(emoji).char_limit(8)).labelled_by(label.id);
	ui.add_space(12.0);
	let label = crate::dialog::label(ui, "server-sounds-volume");
	design::slider(ui, volume, 0..=100, "%").labelled_by(label.id);
	ui.add_space(8.0);
}

fn heading(ui: &mut egui::Ui, key: &str, width: f32) {
	ui.allocate_ui_with_layout(
		egui::vec2(width, 32.0),
		egui::Layout::left_to_right(egui::Align::Center),
		|ui| {
			ui.set_width(width);
			ui.add(
				egui::Label::new(design::semibold(ui, crate::i18n::translate(key), 11.0))
					.truncate(),
			);
		},
	);
}

/// Unchanged text keeps the current emoji (including a custom one); clearing removes it.
fn emoji_patch(emoji: &str, original: &str, custom: bool) -> Patch<String> {
	if emoji == original {
		Patch::Absent
	} else if emoji.is_empty() {
		if custom { Patch::Absent } else { Patch::Null }
	} else {
		Patch::Value(emoji.to_owned())
	}
}

fn valid_fields(name: &str, emoji: &str) -> bool {
	model::server_admin::valid_sound_fields(name.trim(), emoji.trim())
}

#[cfg(test)]
mod tests {
	use super::*;
	fn upload(duration: f32) -> Upload {
		Upload {
			name: "Air horn".into(),
			emoji: String::new(),
			volume: 100,
			duration,
			peaks: vec![128; (duration / 10.0) as usize],
			original: None,
			start: 0.0,
			end: duration.min(MAX_MS),
			view: (0.0, duration),
			target: (0.0, duration),
			held: None,
			trim: Trim::Idle,
			preview: None,
			playing: None,
		}
	}
	#[test]
	fn trim_selection_stays_within_the_file_and_the_length_limits() {
		let mut clip = upload(77_800.0);
		assert_eq!((clip.start, clip.end), (0.0, MAX_MS));
		assert!(!clip.whole());
		// Dragging an edge past the limit carries the other edge along.
		clip.drag(Grab::End, 30_000.0, 0.0);
		assert_eq!((clip.start, clip.end), (30_000.0 - MAX_MS, 30_000.0));
		clip.drag(Grab::Start, 29_950.0, 0.0);
		assert_eq!((clip.start, clip.end), (29_950.0, 29_950.0 + MIN_MS));
		clip.drag(Grab::Start, 10_000.0, 0.0);
		assert_eq!((clip.start, clip.end), (10_000.0, 10_000.0 + MAX_MS));
		clip.drag(Grab::Start, 90_000.0, 0.0);
		assert_eq!((clip.start, clip.end), (77_600.0, 77_800.0));
		clip.drag(Grab::End, -50.0, 0.0);
		assert_eq!((clip.start, clip.end), (0.0, MIN_MS));
		// Moving the span keeps its length and stops at both ends.
		clip.drag(Grab::End, 3000.0, 0.0);
		clip.drag(Grab::Body, 50_500.0, 500.0);
		assert_eq!((clip.start, clip.end), (50_000.0, 53_000.0));
		clip.drag(Grab::Body, 99_000.0, 500.0);
		assert_eq!((clip.start, clip.end), (74_800.0, 77_800.0));
		clip.drag(Grab::Body, -99_000.0, 500.0);
		assert_eq!((clip.start, clip.end), (0.0, 3000.0));

		let mut short = upload(1500.0);
		assert!(short.whole());
		short.drag(Grab::Start, 400.0, 0.0);
		assert!(!short.whole());
		assert_eq!((short.start, short.end), (400.0, 1500.0));
	}

	#[test]
	fn upload_review_uses_the_original_only_for_an_untrimmed_file_and_encodes_once() {
		let mut ui = SoundsUi::default();
		// A result without a matching request is ignored.
		ui.accept(Ok(Some(("Tone".into(), 1500, vec![1; 150], None))));
		assert!(ui.upload.is_none());
		ui.choose();
		assert!(ui.take_request() && !ui.take_request());
		ui.accept(Ok(Some(("Tone".into(), 0, vec![], None))));
		assert!(ui.upload.is_none() && ui.error.is_some());
		ui.choose();
		assert!(ui.take_request());
		ui.accept(Ok(Some(("Tone".into(), 77_800, vec![9; 7780], None))));
		assert!(ui.has_changes());
		assert!(ui.take_trim().is_none(), "nothing is encoded before Upload");
		ui.upload.as_mut().unwrap().trim = Trim::Requested;
		assert_eq!(ui.take_trim(), Some((0, 5200)));
		assert!(ui.take_trim().is_none());
		ui.accept_trim(Ok(vec![0; model::server_admin::MAX_SOUND_FILE_BYTES + 1]));
		assert!(matches!(ui.upload.as_ref().unwrap().trim, Trim::Idle));
		assert!(ui.error.is_some());
		ui.upload.as_mut().unwrap().trim = Trim::Requested;
		ui.take_trim();
		ui.accept_trim(Ok(b"OggS".to_vec()));
		assert!(matches!(ui.upload.as_ref().unwrap().trim, Trim::Ready(_)));
		// A late result never replaces a prepared file.
		ui.accept_trim(Err("late"));
		assert!(matches!(ui.upload.as_ref().unwrap().trim, Trim::Ready(_)));

		let clip = ui.upload.as_mut().unwrap();
		clip.playing = Some(1.0);
		clip.stop_preview();
		assert_eq!(ui.take_preview(), Some(None));
		assert_eq!(ui.take_preview(), None);
		assert!(!ui.take_closed());
	}

	#[test]
	fn waveform_handles_move_only_while_dragged_and_zoom_after_a_still_hold() {
		fn frame(ctx: &egui::Context, clip: &mut Upload, time: f64, events: Vec<egui::Event>) {
			let output = ctx.run_ui(
				egui::RawInput {
					screen_rect: Some(egui::Rect::from_min_size(
						egui::Pos2::ZERO,
						egui::vec2(500.0, 200.0),
					)),
					time: Some(time),
					events,
					..Default::default()
				},
				|ui| waveform(ui, clip, true),
			);
			output.drop_without_applying_deltas();
		}
		let button = |pos: egui::Pos2, pressed: bool| egui::Event::PointerButton {
			pos,
			button: egui::PointerButton::Primary,
			pressed,
			modifiers: egui::Modifiers::NONE,
		};
		let ctx = egui::Context::default();
		let mut clip = upload(77_800.0);
		frame(&ctx, &mut clip, 0.0, vec![]);
		frame(&ctx, &mut clip, 0.1, vec![]);
		// Hovering the waveform, with no button held, changes nothing.
		for (index, x) in [120.0, 200.0, 300.0, 90.0].into_iter().enumerate() {
			let pos = egui::pos2(x, 50.0);
			frame(
				&ctx,
				&mut clip,
				0.2 + index as f64 * 0.1,
				vec![egui::Event::PointerMoved(pos)],
			);
		}
		assert_eq!((clip.start, clip.end), (0.0, MAX_MS));
		assert!(clip.held.is_none());

		// Drag the end handle to the right: past 5.2 seconds the start follows.
		// The end handle of the initial 5.2-second selection.
		let grab = egui::pos2(99.0, 50.0);
		frame(&ctx, &mut clip, 1.0, vec![egui::Event::PointerMoved(grab)]);
		frame(&ctx, &mut clip, 1.1, vec![button(grab, true)]);
		for (index, x) in [110.0, 150.0, 200.0].into_iter().enumerate() {
			frame(
				&ctx,
				&mut clip,
				1.2 + index as f64 * 0.1,
				vec![egui::Event::PointerMoved(egui::pos2(x, 50.0))],
			);
		}
		assert!(
			clip.held
				.as_ref()
				.is_some_and(|held| held.grab == Grab::End)
		);
		assert!(clip.start > 10_000.0, "{}", clip.start);
		assert!((clip.end - clip.start - MAX_MS).abs() < 1.0);
		// Held for a second with only small jitters on a long file: the view zooms in around
		// the handle.
		assert_eq!(clip.target, (0.0, clip.duration));
		for (time, x) in [(1.6, 201.5), (2.0, 199.0)] {
			frame(
				&ctx,
				&mut clip,
				time,
				vec![egui::Event::PointerMoved(egui::pos2(x, 50.0))],
			);
			assert_eq!(clip.target, (0.0, clip.duration));
		}
		let moved = (clip.start, clip.end);
		frame(&ctx, &mut clip, 2.6, vec![]);
		assert!((clip.target.1 - clip.target.0 - ZOOM_SPAN_MS).abs() < 1.0);
		assert_eq!(
			(clip.start, clip.end),
			moved,
			"zooming never moves the selection"
		);
		for step in 0..40 {
			frame(&ctx, &mut clip, 2.7 + f64::from(step) * 0.05, vec![]);
		}
		assert_eq!(clip.view, clip.target);
		assert_eq!((clip.start, clip.end), moved);
		// Releasing zooms back out and leaves the selection in place.
		frame(
			&ctx,
			&mut clip,
			5.0,
			vec![button(egui::pos2(199.0, 50.0), false)],
		);
		frame(&ctx, &mut clip, 5.1, vec![]);
		assert!(clip.held.is_none());
		assert_eq!(clip.target, (0.0, clip.duration));
		assert_eq!((clip.start, clip.end), moved);
	}

	#[test]
	fn sound_fields_are_bounded_and_emoji_edits_preserve_unchanged_or_custom_emoji() {
		assert!(valid_fields(" Air horn ", ""));
		assert!(valid_fields("Air horn", "x"));
		assert!(!valid_fields("x", ""));
		assert!(!valid_fields(&"x".repeat(33), ""));
		assert!(!valid_fields("Air horn", "two words"));
		assert!(!valid_fields("Air\nhorn", ""));

		assert_eq!(emoji_patch("a", "a", false), Patch::Absent);
		assert_eq!(emoji_patch("", "a", false), Patch::Null);
		assert_eq!(emoji_patch("b", "a", false), Patch::Value("b".into()));
		// A custom emoji is not editable here; an empty field leaves it in place.
		assert_eq!(emoji_patch("", "", true), Patch::Absent);
		assert_eq!(emoji_patch("b", "", true), Patch::Value("b".into()));
	}
}
