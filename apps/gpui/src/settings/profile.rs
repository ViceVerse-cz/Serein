//! Profile: display name, pronouns, About Me, profile colour and picture with a live preview
//! card and a Save/Cancel bar, following the main app's `profile_edit::Editor`. The draft is
//! session-only; only changed fields are sent, through `State::save_own_profile`.
use super::{format_hex, kit, parse_hex};
use crate::theme::{Icon, color, icon, palette, tint};
use crate::{Serein, input::Input, sidebar::avatar};
use client_core::{Event, State, auth::Failure};
use gpui::{prelude::*, *};
use model::{Id, ProfileEdit, UserProfile};
use std::{io::Cursor, path::Path, sync::Arc};

/// Largest picture file read from disk, as the main app.
const MAX_PICTURE_FILE: u64 = 8 * 1024 * 1024;
/// Edge of the square PNG that is uploaded, as the main app.
const PICTURE_EDGE: u32 = 256;
/// Side-by-side form and preview from this content width, as the main app.
const WIDE: f32 = 620.;

#[derive(Clone, PartialEq, Eq, Debug)]
pub(crate) struct Draft {
	name: String,
	bio: String,
	pronouns: String,
	color: Option<u32>,
	/// `Some(Some(uri))` uploads a new picture, `Some(None)` removes the current one.
	avatar: Option<Option<String>>,
}
impl Draft {
	fn from_profile(profile: &UserProfile) -> Self {
		Self {
			name: profile.global_name.clone().unwrap_or_default(),
			bio: profile.bio.clone(),
			pronouns: profile.pronouns.clone(),
			color: profile.accent_color,
			avatar: None,
		}
	}
	/// Only the fields that differ from the loaded profile; an empty name clears it.
	fn changes(&self, profile: &UserProfile) -> ProfileEdit {
		let name = (!self.name.is_empty()).then(|| self.name.clone());
		ProfileEdit {
			global_name: (name != profile.global_name).then_some(name),
			bio: (self.bio != profile.bio).then(|| self.bio.clone()),
			pronouns: (self.pronouns != profile.pronouns).then(|| self.pronouns.clone()),
			accent_color: (self.color != profile.accent_color).then_some(self.color),
			avatar: match &self.avatar {
				// Removing a picture the account does not have is not a change.
				Some(None) if profile.user.avatar.is_none() => None,
				other => other.clone(),
			},
		}
	}
	/// After a reload, untouched fields adopt the fresh values; edited ones are kept.
	fn rebase(&mut self, before: &Self, fresh: &Self) {
		if self.name == before.name {
			self.name = fresh.name.clone();
		}
		if self.bio == before.bio {
			self.bio = fresh.bio.clone();
		}
		if self.pronouns == before.pronouns {
			self.pronouns = fresh.pronouns.clone();
		}
		if self.color == before.color {
			self.color = fresh.color;
		}
	}
}

/// The profile page's draft. Text lives in the inputs; colour and picture choices here.
pub struct ProfileEditor {
	generation: Option<u64>,
	name: Entity<Input>,
	pronouns: Entity<Input>,
	bio: Entity<Input>,
	/// Profile colour as `#RRGGBB`, applied while typing once it parses.
	hex: Entity<Input>,
	color: Option<u32>,
	avatar: Option<Option<String>>,
	/// The profile the draft was last rebased on.
	baseline: Option<Draft>,
	submitted: Option<u64>,
	saved: bool,
	/// A native picker is open for this `revision`.
	choosing: bool,
	revision: u64,
	pending_picture: Option<Arc<RenderImage>>,
	/// Local copy of the last saved picture, keyed by the avatar hash the service returned.
	saved_picture: Option<(Option<String>, Arc<RenderImage>)>,
}

impl ProfileEditor {
	pub fn new(window: &mut Window, cx: &mut Context<Serein>) -> Self {
		let field = |placeholder: &str, limit: usize, cx: &mut Context<Serein>| {
			let input = cx.new(Input::new);
			input.update(cx, |input, cx| {
				input.set_placeholder(placeholder.into(), cx)
			});
			cx.subscribe_in(
				&input,
				window,
				move |this, input, event: &crate::input::Event, window, cx| match event {
					crate::input::Event::Changed => {
						// Same character limits as the main app's text edits.
						let value = input.read(cx).value();
						if value.chars().count() > limit {
							let value = value.chars().take(limit).collect::<String>();
							input.update(cx, |input, cx| input.set_value(value, cx));
						}
						this.settings.profile.saved = false;
						cx.notify();
					}
					crate::input::Event::Cancel => this.close_settings(window, cx),
					_ => {}
				},
			)
			.detach();
			input
		};
		let name = field("Your display name", model::MAX_PROFILE_NAME_CHARS, cx);
		let pronouns = field("Add your pronouns", model::MAX_PROFILE_PRONOUNS_CHARS, cx);
		let bio = field(
			"Tell people about yourself",
			model::MAX_PROFILE_BIO_CHARS,
			cx,
		);
		let hex = cx.new(Input::new);
		hex.update(cx, |input, cx| input.set_placeholder("#1A72E8".into(), cx));
		cx.subscribe_in(
			&hex,
			window,
			|this, hex, event: &crate::input::Event, window, cx| match event {
				crate::input::Event::Changed => {
					if let Some([r, g, b]) = parse_hex(hex.read(cx).value().trim()) {
						let editor = &mut this.settings.profile;
						editor.color = Some(u32::from_be_bytes([0, r, g, b]));
						editor.saved = false;
						cx.notify();
					}
				}
				crate::input::Event::Cancel => this.close_settings(window, cx),
				_ => {}
			},
		)
		.detach();
		Self {
			generation: None,
			name,
			pronouns,
			bio,
			hex,
			color: None,
			avatar: None,
			baseline: None,
			submitted: None,
			saved: false,
			choosing: false,
			revision: 0,
			pending_picture: None,
			saved_picture: None,
		}
	}

	fn draft(&self, cx: &App) -> Draft {
		Draft {
			name: self.name.read(cx).value().to_owned(),
			bio: self.bio.read(cx).value().to_owned(),
			pronouns: self.pronouns.read(cx).value().to_owned(),
			color: self.color,
			avatar: self.avatar.clone(),
		}
	}

	fn set_draft(&mut self, draft: Draft, cx: &mut App) {
		for (input, value) in [
			(&self.name, draft.name),
			(&self.bio, draft.bio),
			(&self.pronouns, draft.pronouns),
		] {
			if input.read(cx).value() != value {
				input.update(cx, |input, cx| input.set_value(value, cx));
			}
		}
		self.color = draft.color;
		self.avatar = draft.avatar;
	}

	/// Forget the draft, e.g. for a new session.
	fn reset(&mut self, generation: u64, cx: &mut App) {
		self.set_draft(
			Draft {
				name: String::new(),
				bio: String::new(),
				pronouns: String::new(),
				color: None,
				avatar: None,
			},
			cx,
		);
		*self = Self {
			generation: Some(generation),
			name: self.name.clone(),
			pronouns: self.pronouns.clone(),
			bio: self.bio.clone(),
			hex: self.hex.clone(),
			color: None,
			avatar: None,
			baseline: None,
			submitted: None,
			saved: false,
			choosing: false,
			revision: self.revision,
			pending_picture: None,
			saved_picture: None,
		};
	}
}

/// What the preview paints inside the avatar circle.
enum Picture {
	/// The account's current picture (or initials while it is not loaded).
	Remote,
	/// A chosen or just-saved local image.
	Local(Arc<RenderImage>),
	/// Removal is pending, so the initials placeholder is shown.
	Removed,
}

impl Serein {
	pub(super) fn settings_profile(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Div {
		let p = palette();
		if self.settings.profile.generation != Some(self.state.generation) {
			let generation = self.state.generation;
			self.settings.profile.reset(generation, cx);
		}
		let own = &self.state.own_profile;
		if own.data.is_none() && !own.loading && own.error.is_none() {
			let command = self.state.load_own_profile();
			self.dispatch(command);
		}
		let editor = &mut self.settings.profile;
		let own = &self.state.own_profile;
		if editor.submitted == Some(own.request) && !own.saving {
			editor.submitted = None;
			if own.error.is_none()
				&& let Some(profile) = &own.data
			{
				let uploaded = matches!(editor.avatar, Some(Some(_)));
				editor.saved_picture = match (uploaded, editor.pending_picture.take()) {
					(true, Some(image)) => Some((profile.user.avatar.clone(), image)),
					_ => None,
				};
				editor.set_draft(Draft::from_profile(profile), cx);
				editor.saved = true;
			}
		}
		let own = &self.state.own_profile;
		let mut page = div().flex().flex_col().gap_3();
		if own.loading {
			page = page.child(kit::hint("Loading your profile…"));
		}
		if let Some(error) = own.error {
			let reload = !own.loading && !own.saving;
			page = page
				.child(kit::notice(kit::Level::Warning, error))
				.when(reload, |d| {
					d.child(div().flex().child(
						kit::text_action("profile-reload", "Reload profile").on_click(cx.listener(
							|this, _, _, cx| {
								let command = this.state.load_own_profile();
								this.dispatch(command);
								cx.notify();
							},
						)),
					))
				});
		}
		let Some(profile) = own.data.clone() else {
			return page;
		};
		let saving = own.saving;
		let editable = !own.loading && !saving;
		let fresh = Draft::from_profile(&profile);
		let editor = &mut self.settings.profile;
		match editor.baseline.take() {
			Some(before) if before != fresh => {
				let mut draft = editor.draft(cx);
				draft.rebase(&before, &fresh);
				editor.set_draft(draft, cx);
			}
			Some(_) => {}
			None => editor.set_draft(fresh.clone(), cx),
		}
		editor.baseline = Some(fresh);
		// Show the chosen colour in the hex field unless it is being edited.
		let hex_focused = editor.hex.read(cx).focus_handle(cx).is_focused(window);
		if !hex_focused && let Some(value) = editor.color {
			let text = format_hex(rgb_bytes(value));
			if editor.hex.read(cx).value() != text {
				editor.hex.update(cx, |input, cx| input.set_value(text, cx));
			}
		}
		let draft = editor.draft(cx);
		let changes = draft.changes(&profile);
		let changed = changes != ProfileEdit::default();
		let valid = changes.valid();
		let picture = match &draft.avatar {
			Some(Some(_)) => editor
				.pending_picture
				.clone()
				.map_or(Picture::Remote, Picture::Local),
			Some(None) => Picture::Removed,
			None => match &editor.saved_picture {
				Some((hash, image)) if hash.is_some() && *hash == profile.user.avatar => {
					Picture::Local(image.clone())
				}
				_ => Picture::Remote,
			},
		};
		let choosing = editor.choosing;
		let saved = editor.saved;
		let form = self.profile_form(&draft, &profile, editable, window, cx);
		let preview = profile_preview(&draft, &profile, picture, editable && !choosing, cx);
		let wide = content_width(window) >= WIDE;
		let demo = self.state.demo;
		let can_save = changed && valid && self.state.can_save_own_profile();
		let status = if saving {
			Some(("Saving profile…", p.muted))
		} else if saved {
			Some((
				if demo {
					"Saved in preview"
				} else {
					"Profile saved"
				},
				p.positive,
			))
		} else if changed {
			Some(("You have unsaved changes.", p.text))
		} else {
			None
		};
		let bar =
			div()
				.w_full()
				.mt_1()
				.px_3()
				.py_2()
				.rounded(px(8.))
				.bg(color(p.base))
				.flex()
				.items_center()
				.gap_3()
				.child(
					div().flex_1().min_w_0().text_size(px(13.)).children(
						status.map(|(text, tone)| div().text_color(color(tone)).child(text)),
					),
				)
				.child(
					kit::button(
						"profile-cancel",
						"Cancel",
						kit::ButtonKind::Outline,
						changed && editable,
					)
					.when(changed && editable, |d| {
						d.on_click(cx.listener(|this, _, _, cx| {
							if let Some(profile) = &this.state.own_profile.data {
								let editor = &mut this.settings.profile;
								editor.set_draft(Draft::from_profile(profile), cx);
								editor.pending_picture = None;
								editor.saved = false;
							}
							cx.notify();
						}))
					}),
				)
				.child(
					kit::button(
						"profile-save",
						"Save changes",
						kit::ButtonKind::Primary,
						can_save,
					)
					.when(can_save, |d| {
						d.on_click(cx.listener(|this, _, _, cx| this.save_profile(cx)))
					}),
				);
		page = page.when(!valid, |d| {
			d.child(kit::notice(
				kit::Level::Warning,
				"Check character limits and remove control characters. A display name cannot contain only spaces.",
			))
		});
		page = if wide {
			page.child(
				div()
					.w_full()
					.flex()
					.items_start()
					.gap_6()
					.child(div().flex_1().min_w_0().child(form))
					.child(div().w(px(300.)).flex_none().child(preview)),
			)
			.child(bar)
		} else {
			page.child(form).child(bar).child(preview)
		};
		page.when(!demo && !self.state.gateway_connected, |d| {
			d.child(kit::hint("Reconnect to save your profile."))
		})
	}

	fn profile_form(
		&mut self,
		draft: &Draft,
		profile: &UserProfile,
		editable: bool,
		window: &Window,
		cx: &mut Context<Self>,
	) -> Div {
		let p = palette();
		let editor = &self.settings.profile;
		let choosing = editor.choosing;
		let has_picture = match &draft.avatar {
			Some(value) => value.is_some(),
			None => profile.user.avatar.is_some(),
		};
		let picture_detail = match &draft.avatar {
			Some(Some(_)) => "New picture chosen. Save to upload it.",
			Some(None) => "Your picture will be removed when you save.",
			None => "PNG, JPEG, GIF or WebP up to 8 MB. Cropped to a square.",
		};
		let picture = kit::row(
			"Profile picture",
			Some(picture_detail),
			div()
				.flex()
				.items_center()
				.gap_1()
				.when(choosing, |d| {
					d.child(
						div()
							.text_size(px(12.))
							.text_color(color(p.muted))
							.child("Choosing…"),
					)
				})
				.map(|d| {
					if draft.avatar.is_some() {
						d.child(kit::text_action("profile-picture-undo", "Undo").on_click(
							cx.listener(|this, _, _, cx| {
								let editor = &mut this.settings.profile;
								editor.avatar = None;
								editor.pending_picture = None;
								cx.notify();
							}),
						))
					} else if has_picture {
						d.child(
							kit::text_action("profile-picture-remove", "Remove").on_click(
								cx.listener(|this, _, _, cx| {
									let editor = &mut this.settings.profile;
									editor.avatar = Some(None);
									editor.pending_picture = None;
									editor.saved = false;
									cx.notify();
								}),
							),
						)
					} else {
						d
					}
				})
				.child(
					kit::button(
						"profile-picture-change",
						"Change",
						kit::ButtonKind::Outline,
						!choosing,
					)
					.when(!choosing, |d| {
						d.on_click(cx.listener(|this, _, _, cx| this.pick_profile_picture(cx)))
					}),
				),
		);
		let colour = kit::row(
			"Profile color",
			Some("Tints your banner when you have not set a banner image."),
			match draft.color {
				Some(value) => {
					let focused = editor.hex.read(cx).focus_handle(cx).is_focused(window);
					div()
						.flex()
						.items_center()
						.gap_2()
						.child(
							kit::text_action("profile-color-default", "Use default").on_click(
								cx.listener(|this, _, _, cx| {
									this.settings.profile.color = None;
									this.settings.profile.saved = false;
									cx.notify();
								}),
							),
						)
						.child(
							text_box(focused)
								.w(px(96.))
								.h(px(32.))
								.child(editor.hex.clone()),
						)
						.child(
							div()
								.w(px(40.))
								.h(px(28.))
								.rounded(px(6.))
								.border_1()
								.border_color(color(p.border))
								.bg(rgb(value)),
						)
				}
				None => div().child(
					kit::text_action("profile-color-custom", "Custom color").on_click(cx.listener(
						|this, _, _, cx| {
							this.settings.profile.color = Some(ui::design::DEFAULT_PRIMARY_RGB);
							this.settings.profile.saved = false;
							cx.notify();
						},
					)),
				),
			},
		);
		let field = |label: &str, input: &Entity<Input>, counter: Option<usize>, tall: bool| {
			let focused = input.read(cx).focus_handle(cx).is_focused(window);
			let count = input.read(cx).value().chars().count();
			div()
				.flex()
				.flex_col()
				.gap_1()
				.child(
					div()
						.flex()
						.items_center()
						.justify_between()
						.child(kit::eyebrow(label))
						.children(counter.map(|limit| {
							div()
								.text_size(px(11.))
								.text_color(color(p.muted))
								.child(format!("{count} / {limit}"))
						})),
				)
				.child(
					text_box(focused)
						.w_full()
						.when(tall, |d| d.min_h(px(104.)).items_start().py_1())
						.when(!tall, |d| d.h(px(38.)))
						.child(input.clone()),
				)
		};
		let form = div()
			.relative()
			.flex()
			.flex_col()
			.gap_2()
			.child(picture)
			.child(div().h_1())
			.child(field("Display name", &editor.name, None, false))
			.child(kit::hint("Leave blank to use your username."))
			.child(div().h_1())
			.child(field("Pronouns", &editor.pronouns, None, false))
			.child(div().h_1())
			.child(field(
				"About Me",
				&editor.bio,
				Some(model::MAX_PROFILE_BIO_CHARS),
				true,
			))
			.child(kit::hint("Shift+Enter starts a new line."))
			.child(div().h_1())
			.child(colour);
		// Loading or saving: dim the form and swallow clicks, like `add_enabled_ui`.
		form.when(!editable, |d| {
			d.opacity(0.6)
				.child(div().id("profile-form-busy").absolute().inset_0().occlude())
		})
	}

	fn save_profile(&mut self, cx: &mut Context<Self>) {
		let Some(profile) = self.state.own_profile.data.as_ref() else {
			return;
		};
		let changes = self.settings.profile.draft(cx).changes(profile);
		if let Some(command) = self.state.save_own_profile(changes) {
			self.settings.profile.submitted = Some(self.state.own_profile.request);
			self.settings.profile.saved = false;
			self.dispatch(Some(command));
		}
		cx.notify();
	}

	/// Opens the native picker; the file is read, cropped and encoded off the UI thread.
	fn pick_profile_picture(&mut self, cx: &mut Context<Self>) {
		let Some(user) = self.state.user.as_ref().map(|user| user.id) else {
			return;
		};
		let editor = &mut self.settings.profile;
		if editor.choosing {
			return;
		}
		editor.revision = editor.revision.wrapping_add(1);
		editor.choosing = true;
		let scope = (self.state.generation, user, editor.revision);
		let chosen = cx.prompt_for_paths(PathPromptOptions {
			files: true,
			directories: false,
			multiple: false,
			prompt: Some("Choose".into()),
		});
		cx.spawn(async move |this, cx| {
			let path = match chosen.await {
				Ok(Ok(Some(mut paths))) => paths.pop(),
				Ok(Err(_)) => {
					let _ = this.update(cx, |this, cx| {
						this.accept_profile_picture(
							scope,
							Err("Could not open the file picker"),
							cx,
						)
					});
					return;
				}
				_ => None,
			};
			let result = match path {
				Some(path) => cx
					.background_executor()
					.spawn(async move { read_picture(&path) })
					.await
					.map(Some),
				None => Ok(None),
			};
			let _ = this.update(cx, |this, cx| {
				this.accept_profile_picture(scope, result, cx)
			});
		})
		.detach();
		cx.notify();
	}

	fn accept_profile_picture(
		&mut self,
		scope: (u64, Id, u64),
		result: Result<Option<(String, Arc<RenderImage>)>, &'static str>,
		cx: &mut Context<Self>,
	) {
		let editor = &mut self.settings.profile;
		let current = (
			self.state.generation,
			self.state.user.as_ref().map_or(Id(0), |user| user.id),
			editor.revision,
		);
		if !editor.choosing || scope != current {
			return;
		}
		editor.choosing = false;
		match result {
			Ok(Some((uri, image))) => {
				editor.avatar = Some(Some(uri));
				editor.pending_picture = Some(image);
				editor.saved = false;
			}
			Ok(None) => {}
			Err(error) => self.notify_user(error),
		}
		cx.notify();
	}
}

/// Content column width inside the settings modal, as `render_settings` lays it out.
fn content_width(window: &Window) -> f32 {
	let modal = (f32::from(window.viewport_size().width) - 32.).clamp(280., 1100.);
	// Sidebar, page padding and the scroll gutter.
	(modal - 232. - 40. - 20. - 8.).min(712.)
}

fn rgb_bytes(value: u32) -> [u8; 3] {
	let [_, r, g, b] = value.to_be_bytes();
	[r, g, b]
}

/// Input chrome shared by the text fields.
fn text_box(focused: bool) -> Div {
	let p = palette();
	div()
		.px_2()
		.rounded(px(6.))
		.bg(color(p.base))
		.border_1()
		.border_color(color(if focused { p.accent } else { p.border }))
		.flex()
		.items_center()
		.text_size(px(15.))
}

/// The card others see, updated as you type.
fn profile_preview(
	draft: &Draft,
	profile: &UserProfile,
	picture: Picture,
	clickable: bool,
	cx: &mut Context<Serein>,
) -> Div {
	let p = palette();
	let name = if draft.name.is_empty() {
		profile.username.clone()
	} else {
		draft.name.clone()
	};
	let banner_image = profile
		.banner_key()
		.and_then(|key| crate::images::get(&key));
	let banner = div().w_full().h(px(90.)).map(|d| match banner_image {
		Some(image) => d.child(img(image).size_full().object_fit(ObjectFit::Cover)),
		None => d.bg(draft.color.map_or(tint(p.accent, 0.4), rgb)),
	});
	let face = match picture {
		Picture::Local(image) => div().size(px(80.)).rounded_full().overflow_hidden().child(
			img(image)
				.size_full()
				.rounded_full()
				.object_fit(ObjectFit::Cover),
		),
		Picture::Removed => avatar(&profile.user.name, 80., None),
		Picture::Remote => avatar(&profile.user.name, 80., Some(&profile.user)),
	};
	let portrait = div()
		.id("profile-preview-avatar")
		.group("profile-preview-avatar")
		.absolute()
		.left(px(10.))
		.top(px(44.))
		.p(px(6.))
		.rounded_full()
		.bg(color(p.raised))
		.child(div().relative().child(face).when(clickable, |d| {
			d.child(
				div()
					.absolute()
					.inset_0()
					.rounded_full()
					.bg(hsla(0., 0., 0., 0.47))
					.flex()
					.items_center()
					.justify_center()
					.opacity(0.)
					.group_hover("profile-preview-avatar", |d| d.opacity(1.))
					.child(icon(Icon::Pencil, px(22.), white())),
			)
		}))
		.when(clickable, |d| {
			d.cursor_pointer()
				.tooltip(crate::tooltip("Change profile picture"))
				.on_click(cx.listener(|this, _, _, cx| this.pick_profile_picture(cx)))
		});
	let about = (!draft.bio.is_empty()).then(|| {
		div()
			.flex()
			.flex_col()
			.gap_1()
			.child(div().my_2().h(px(1.)).bg(color(p.border)))
			.child(
				div()
					.text_size(px(12.))
					.font_weight(FontWeight::SEMIBOLD)
					.text_color(color(p.text_strong))
					.child("ABOUT ME"),
			)
			.child(
				div()
					.text_size(px(14.))
					.line_height(px(19.))
					.text_color(color(p.text))
					.children(draft.bio.lines().map(|line| div().child(line.to_owned()))),
			)
	});
	div()
		.flex()
		.flex_col()
		.gap_1()
		.child(kit::eyebrow("Preview"))
		.child(
			div()
				.w_full()
				.relative()
				.rounded(px(8.))
				.bg(color(p.raised))
				.border_1()
				.border_color(color(p.border))
				.overflow_hidden()
				.flex()
				.flex_col()
				.child(banner)
				.child(portrait)
				.child(div().h(px(62.)))
				.child(
					div().px_3().pb_3().child(
						div()
							.w_full()
							.min_h(px(138.))
							.p_3()
							.rounded(px(8.))
							.bg(color(p.chat))
							.flex()
							.flex_col()
							.gap_1()
							.child(
								div()
									.text_size(px(20.))
									.font_weight(FontWeight::SEMIBOLD)
									.text_color(color(p.text_strong))
									.child(name),
							)
							.child(
								div()
									.text_size(px(13.))
									.text_color(color(p.text))
									.child(profile.username.clone()),
							)
							.when(!draft.pronouns.is_empty(), |d| {
								d.child(
									div()
										.text_size(px(12.))
										.text_color(color(p.muted))
										.child(draft.pronouns.clone()),
								)
							})
							.children(about),
					),
				),
		)
}

/// Reads one chosen picture within the main app's limits.
fn read_picture(path: &Path) -> Result<(String, Arc<RenderImage>), &'static str> {
	let metadata =
		std::fs::symlink_metadata(path).map_err(|_| "Could not open the chosen image")?;
	if !metadata.is_file() {
		return Err("Choose a regular image file");
	}
	if metadata.len() == 0 || metadata.len() > MAX_PICTURE_FILE {
		return Err("Choose an image up to 8 MB");
	}
	let bytes = std::fs::read(path).map_err(|_| "Could not read the chosen image")?;
	let (uri, pixels) = prepare_picture(&bytes)?;
	let mut pixels = pixels;
	// GPUI's atlas takes BGRA.
	for pixel in pixels.as_chunks_mut::<4>().0 {
		pixel.swap(0, 2);
	}
	Ok((
		uri,
		Arc::new(RenderImage::new(vec![image::Frame::new(pixels)])),
	))
}

/// Decodes, centre-crops to a square, shrinks to 256 px and encodes the PNG data URI the
/// profile endpoint accepts.
fn prepare_picture(bytes: &[u8]) -> Result<(String, image::RgbaImage), &'static str> {
	use image::GenericImageView;
	if bytes.len() as u64 > MAX_PICTURE_FILE {
		return Err("Choose an image up to 8 MB");
	}
	let mut reader = image::ImageReader::new(Cursor::new(bytes))
		.with_guessed_format()
		.map_err(|_| "Choose a PNG, JPEG, GIF or WebP image")?;
	let mut limits = image::Limits::default();
	limits.max_image_width = Some(4096);
	limits.max_image_height = Some(4096);
	limits.max_alloc = Some(64 * 1024 * 1024);
	reader.limits(limits);
	let image = reader
		.decode()
		.map_err(|_| "Image is unsupported or too large; use at most 4096 × 4096 pixels")?;
	let edge = image.width().min(image.height());
	let crop = image.view(
		(image.width() - edge) / 2,
		(image.height() - edge) / 2,
		edge,
		edge,
	);
	let side = edge.min(PICTURE_EDGE);
	let pixels = image::imageops::thumbnail(&*crop, side, side);
	let mut png = Cursor::new(Vec::new());
	image::DynamicImage::ImageRgba8(pixels.clone())
		.write_to(&mut png, image::ImageFormat::Png)
		.map_err(|_| "Could not prepare the picture")?;
	let uri = format!("data:image/png;base64,{}", base64(png.get_ref()));
	if !model::valid_avatar_uri(&uri) {
		return Err("That picture is too large; choose a simpler image");
	}
	Ok((uri, pixels))
}

/// Standard padded base64, enough for one bounded PNG data URI.
fn base64(bytes: &[u8]) -> String {
	const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
	let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
	for chunk in bytes.chunks(3) {
		let byte = |at: usize| u32::from(chunk.get(at).copied().unwrap_or(0));
		let n = (byte(0) << 16) | (byte(1) << 8) | byte(2);
		for i in 0..4 {
			out.push(if i <= chunk.len() {
				char::from(TABLE[(n >> (18 - 6 * i)) as usize & 63])
			} else {
				'='
			});
		}
	}
	out
}

/// Synthetic global profile for the offline preview, like the main app's fixture.
fn synthetic_profile(user: &model::User) -> UserProfile {
	UserProfile {
		user: user.clone(),
		username: "serein.preview".into(),
		global_name: Some(user.name.clone()),
		banner: None,
		accent_color: Some(0x315c68),
		bio: "Building a quieter place for conversations.\nAll details here are synthetic.".into(),
		pronouns: "they / them".into(),
		badges: vec![],
		connections: vec![],
		mutual_guilds: vec![],
		guild: None,
		theme_colors: None,
		clan: None,
		limited: false,
	}
}

/// Offline answer to `Command::EditProfile`: loads the synthetic profile or applies the
/// changes to it in RAM, as the desktop fixture does. Nothing is sent.
pub(crate) fn demo_profile_edit(
	state: &State,
	user: Id,
	request: u64,
	changes: Option<ProfileEdit>,
) -> Event {
	let result = state
		.user
		.as_ref()
		.filter(|own| own.id == user)
		.map(|own| {
			let mut profile = state
				.own_profile
				.data
				.clone()
				.unwrap_or_else(|| synthetic_profile(own));
			if let Some(changes) = changes {
				if !changes.valid() {
					return Err(Failure::Capacity);
				}
				if let Some(name) = changes.global_name {
					profile.user.name = name.clone().unwrap_or_else(|| profile.username.clone());
					profile.global_name = name;
				}
				if let Some(bio) = changes.bio {
					profile.bio = bio;
				}
				if let Some(pronouns) = changes.pronouns {
					profile.pronouns = pronouns;
				}
				if let Some(color) = changes.accent_color {
					profile.accent_color = color;
				}
				if let Some(avatar) = changes.avatar {
					// Synthetic hash: the preview never uploads or fetches images.
					profile.user.avatar =
						avatar.map(|_| "0123456789abcdef0123456789abcdef".to_owned());
				}
			}
			Ok(Box::new(profile))
		})
		.unwrap_or(Err(Failure::Protocol));
	Event::ProfileEdited {
		user,
		request,
		result,
	}
}

#[cfg(test)]
mod tests {
	use super::{Draft, base64, demo_profile_edit, prepare_picture, synthetic_profile};
	use client_core::{Command, Envelope, Event};
	use model::ProfileEdit;
	use std::io::Cursor;

	#[test]
	fn draft_only_sends_changed_fields_and_rebases_untouched_ones() {
		let user = test_support::demo_state().user.unwrap();
		let mut profile = synthetic_profile(&user);
		profile.global_name = Some("Before".into());
		let mut draft = Draft::from_profile(&profile);
		assert!(draft.changes(&profile) == ProfileEdit::default());
		draft.name.clear();
		draft.bio.clear();
		draft.color = None;
		let changes = draft.changes(&profile);
		assert_eq!(changes.global_name, Some(None));
		assert_eq!(changes.bio, Some(String::new()));
		assert_eq!(changes.accent_color, Some(None));
		assert_eq!(changes.pronouns, None);
		assert!(changes.valid());
		// Removing a picture the account does not have is not a change.
		draft.avatar = Some(None);
		assert_eq!(draft.changes(&profile).avatar, None);
		let before = Draft::from_profile(&profile);
		profile.pronouns = "she/her".into();
		draft.rebase(&before, &Draft::from_profile(&profile));
		assert_eq!(draft.pronouns, "she/her");
		assert!(draft.name.is_empty() && draft.bio.is_empty());
		// A name of only spaces is rejected before anything is sent.
		draft.name = "   ".into();
		assert!(!draft.changes(&profile).valid());
	}

	#[test]
	fn demo_loads_and_saves_the_profile_offline() {
		let mut state = test_support::demo_state();
		let Some(Command::EditProfile {
			user,
			request,
			changes: None,
		}) = state.load_own_profile()
		else {
			panic!("load expected")
		};
		let event = demo_profile_edit(&state, user, request, None);
		state.apply(Envelope {
			generation: state.generation,
			event,
		});
		let profile = state.own_profile.data.clone().expect("loaded");
		assert!(state.can_save_own_profile());
		let mut draft = Draft::from_profile(&profile);
		draft.pronouns = "she / her".into();
		draft.name = "Renamed".into();
		let Some(Command::EditProfile {
			user,
			request,
			changes: Some(changes),
		}) = state.save_own_profile(draft.changes(&profile))
		else {
			panic!("save expected")
		};
		let event = demo_profile_edit(&state, user, request, Some(changes));
		assert!(matches!(event, Event::ProfileEdited { result: Ok(_), .. }));
		state.apply(Envelope {
			generation: state.generation,
			event,
		});
		let saved = state.own_profile.data.as_ref().unwrap();
		assert_eq!(saved.pronouns, "she / her");
		assert_eq!(saved.global_name.as_deref(), Some("Renamed"));
		assert_eq!(state.user.as_ref().unwrap().name, "Renamed");
		assert!(!state.own_profile.saving && state.own_profile.error.is_none());
	}

	#[test]
	fn pictures_are_square_bounded_png_data_uris() {
		assert_eq!(base64(b"Man"), "TWFu");
		assert_eq!(base64(b"Ma"), "TWE=");
		assert_eq!(base64(b"M"), "TQ==");
		assert_eq!(base64(b""), "");
		let mut png = Cursor::new(Vec::new());
		image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
			512,
			320,
			image::Rgba([255, 0, 0, 255]),
		))
		.write_to(&mut png, image::ImageFormat::Png)
		.unwrap();
		let (uri, pixels) = prepare_picture(png.get_ref()).unwrap();
		assert_eq!(pixels.dimensions(), (256, 256));
		assert!(model::valid_avatar_uri(&uri));
		assert!(prepare_picture(b"not an image").is_err());
	}
}
