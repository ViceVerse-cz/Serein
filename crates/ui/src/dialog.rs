//! Shared modal chrome.
//!
//! Every dialog in the app — confirmations, channel editors, invite pickers, the settings
//! shells — uses the same surface, header, section padding, footer strip and action buttons
//! from this module. Colours and type come from [`crate::design`]; nothing here introduces a
//! second theme.
//!
//! ```ignore
//! let response = dialog::Dialog::new("delete-channel", "Delete channel?")
//!     .danger()
//!     .show(ctx, |d| {
//!         d.content(|ui| { ui.label("This cannot be undone."); });
//!         d.footer(|ui| {
//!             confirmed = dialog::action(ui, "Delete", dialog::Action::Danger).clicked();
//!             cancelled = dialog::action(ui, "Cancel", dialog::Action::Neutral).clicked();
//!         });
//!     });
//! ```
use crate::{design, icons};
use egui::{Color32, RichText, Stroke};

// Dialog-flavoured names for the shared primitives, so call sites read as dialog chrome.
pub use crate::design::{ButtonKind as Action, button as action};
pub use crate::design::{Level, hint, input, label, notice};

/// Corner radius of a dialog surface.
pub const RADIUS: u8 = 16;
/// Horizontal padding of dialog content. The footer strip bleeds back over it.
pub const PAD: f32 = 20.0;

/// Whether a dialog's confirming action is destructive.
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub enum Tone {
	#[default]
	Neutral,
	Danger,
}

/// The standard dialog surface: raised fill, hairline border, shadow and no inner margin so
/// the footer can run the full width.
pub fn frame(ctx: &egui::Context) -> egui::Frame {
	let colors = design::palette_for(ctx);
	egui::Frame::new()
		.fill(colors.chat.to_opaque())
		.stroke(Stroke::new(1.0, colors.border))
		.corner_radius(RADIUS)
		.shadow(ctx.style_of(ctx.theme()).visuals.window_shadow)
		.inner_margin(0)
}

/// Backdrop that dims the app behind a dialog.
pub fn backdrop(ctx: &egui::Context) -> Color32 {
	let _ = ctx;
	Color32::from_black_alpha(150)
}

/// A modal dialog with the shared header, body and footer chrome.
pub struct Dialog {
	id: egui::Id,
	title: String,
	subtitle: Option<String>,
	tone: Tone,
	icon: Option<icons::Icon>,
	width: f32,
	dismissable: bool,
}

/// Outcome of [`Dialog::show`].
pub struct Response<R> {
	pub inner: R,
	/// The close button, the Escape key or a backdrop click asked to dismiss the dialog.
	pub close: bool,
}

impl Dialog {
	/// `id` only needs to be unique among dialogs; `title` is the header text.
	pub fn new(id: impl std::hash::Hash + std::fmt::Debug, title: impl Into<String>) -> Self {
		Self {
			id: egui::Id::unique(id),
			title: title.into(),
			subtitle: None,
			tone: Tone::Neutral,
			icon: None,
			width: 440.0,
			dismissable: true,
		}
	}
	/// Supporting line under the title. Keep it to one sentence.
	pub fn subtitle(mut self, subtitle: impl Into<String>) -> Self {
		self.subtitle = Some(subtitle.into());
		self
	}
	/// Muted glyph shown before the title, naming what the dialog is about.
	pub fn icon(mut self, icon: icons::Icon) -> Self {
		self.icon = Some(icon);
		self
	}
	/// Marks the dialog destructive: the header gains a tinted warning glyph.
	pub fn danger(mut self) -> Self {
		self.tone = Tone::Danger;
		self
	}
	/// Preferred width; always clamped to the viewport.
	pub fn width(mut self, width: f32) -> Self {
		self.width = width;
		self
	}
	pub fn show<R>(self, ctx: &egui::Context, add: impl FnOnce(&mut Body<'_>) -> R) -> Response<R> {
		let Self {
			id,
			title,
			subtitle,
			tone,
			icon,
			width,
			dismissable,
		} = self;
		let available = ctx.content_rect().size();
		let width = width.min(available.x - 32.0).max(200.0);
		let mut close = false;
		let modal = egui::Modal::new(id)
			.backdrop_color(backdrop(ctx))
			.frame(frame(ctx))
			.show(ctx, |ui| {
				ui.set_width(width);
				ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
				ui.spacing_mut().item_spacing.y = 8.0;
				close |= header(ui, &title, subtitle.as_deref(), tone, icon, dismissable);
				let mut body = Body {
					ui,
					available_height: available.y,
					footer_drawn: false,
				};
				let inner = add(&mut body);
				if !body.footer_drawn {
					body.ui.add_space(PAD);
				}
				inner
			});
		// A centred modal positions itself from the previous pass's size, so the first pass
		// after opening would paint the dialog off-centre. Re-run instead of showing the jump.
		let size = modal.response.rect.size();
		let key = id.with("dialog-size");
		let settled = ctx
			.data(|data| data.get_temp::<egui::Vec2>(key))
			.is_some_and(|previous| (previous - size).length() < 0.5);
		if !settled {
			ctx.data_mut(|data| data.insert_temp(key, size));
			if !ctx.will_discard() {
				ctx.request_discard("dialog layout settling");
			}
		}
		let close = close || modal.should_close();
		Response {
			inner: modal.inner,
			close,
		}
	}
}

/// Cursor inside an open [`Dialog`]: content sections first, then at most one footer.
pub struct Body<'a> {
	ui: &'a mut egui::Ui,
	available_height: f32,
	footer_drawn: bool,
}

impl Body<'_> {
	pub fn available_height(&self) -> f32 {
		self.available_height
	}
	/// Padded content block. Call once per logical section.
	pub fn content<R>(&mut self, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
		egui::Frame::new()
			.inner_margin(egui::Margin {
				left: PAD as i8,
				right: PAD as i8,
				top: 0,
				bottom: 0,
			})
			.show(self.ui, |ui| {
				ui.set_width(ui.available_width());
				add(ui)
			})
			.inner
	}
	/// Padded content that scrolls once it outgrows the viewport. `reserved` is the vertical
	/// space the header and footer need.
	pub fn scroll<R>(&mut self, reserved: f32, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
		let max = (self.available_height - reserved).clamp(120.0, 620.0);
		self.content(|ui| {
			egui::ScrollArea::vertical()
				.max_height(max)
				.auto_shrink([false, true])
				.animated(false)
				.show(ui, |ui| {
					ui.set_width(ui.available_width());
					add(ui)
				})
				.inner
		})
	}
	/// Full-bleed action strip pinned under the content. Add the confirming action first: the
	/// strip lays its children out from the right edge.
	pub fn footer<R>(&mut self, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
		self.footer_drawn = true;
		let colors = design::palette(self.ui);
		self.ui.add_space(PAD - 8.0);
		egui::Frame::new()
			.fill(colors.sidebar.to_opaque())
			.stroke(Stroke::new(1.0, colors.border))
			.corner_radius(egui::CornerRadius {
				nw: 0,
				ne: 0,
				sw: RADIUS,
				se: RADIUS,
			})
			.inner_margin(egui::Margin {
				left: PAD as i8,
				right: PAD as i8,
				top: 16,
				bottom: 16,
			})
			.show(self.ui, |ui| {
				ui.set_width(ui.available_width());
				ui.spacing_mut().item_spacing.x = 8.0;
				ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), add)
					.inner
			})
			.inner
	}
}

/// Title, optional subtitle and the round close control. Returns whether close was clicked.
fn header(
	ui: &mut egui::Ui,
	title: &str,
	subtitle: Option<&str>,
	tone: Tone,
	icon: Option<icons::Icon>,
	dismissable: bool,
) -> bool {
	let colors = design::palette(ui);
	let mut close = false;
	egui::Frame::new()
		.inner_margin(egui::Margin {
			left: PAD as i8,
			right: PAD as i8 - 4,
			top: PAD as i8,
			bottom: 4,
		})
		.show(ui, |ui| {
			ui.set_width(ui.available_width());
			ui.horizontal_top(|ui| {
				if tone == Tone::Danger {
					let (rect, _) =
						ui.allocate_exact_size(egui::Vec2::splat(32.0), egui::Sense::hover());
					ui.painter()
						.rect_filled(rect, 8, colors.danger.gamma_multiply(0.16));
					icons::paint(
						ui.painter(),
						icons::Icon::ShieldWarning,
						rect.shrink(7.0),
						colors.danger,
					);
					ui.add_space(4.0);
				}
				if let Some(icon) = icon.filter(|_| tone != Tone::Danger) {
					let (rect, _) =
						ui.allocate_exact_size(egui::Vec2::splat(30.0), egui::Sense::hover());
					icons::paint(ui.painter(), icon, rect.shrink(3.0), colors.muted);
					ui.add_space(6.0);
				}
				let text_width = (ui.available_width() - 34.0).max(1.0);
				ui.allocate_ui_with_layout(
					egui::vec2(text_width, 0.0),
					egui::Layout::top_down(egui::Align::Min),
					|ui| {
						ui.set_width(text_width);
						ui.spacing_mut().item_spacing.y = 3.0;
						ui.add(
							egui::Label::new(
								design::semibold(ui, title, 19.0).color(colors.text_strong),
							)
							.wrap(),
						);
						if let Some(subtitle) = subtitle {
							ui.add(
								egui::Label::new(
									RichText::new(subtitle).size(13.0).color(colors.muted),
								)
								.wrap(),
							);
						}
					},
				);
				if dismissable {
					ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
						close = icons::button(ui, icons::Icon::Close, 30.0, "Close dialog (Esc)")
							.clicked();
					});
				}
			});
		});
	ui.add_space(8.0);
	close
}

/// Small confirmation dialog: one message and one confirming action.
pub struct Confirm {
	dialog: Dialog,
	message: String,
	confirm: String,
	cancel: String,
	tone: Tone,
	enabled: bool,
	note: Option<(Level, String)>,
}

/// What the user chose in a [`Confirm`].
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Choice {
	Confirmed,
	Cancelled,
}

impl Confirm {
	pub fn new(
		id: impl std::hash::Hash + std::fmt::Debug,
		title: impl Into<String>,
		message: impl Into<String>,
	) -> Self {
		Self {
			dialog: Dialog::new(id, title).width(420.0),
			message: message.into(),
			confirm: "Confirm".to_owned(),
			cancel: "Cancel".to_owned(),
			tone: Tone::Neutral,
			enabled: true,
			note: None,
		}
	}
	/// Label of the confirming action.
	pub fn confirm_label(mut self, label: impl Into<String>) -> Self {
		self.confirm = label.into();
		self
	}
	pub fn cancel_label(mut self, label: impl Into<String>) -> Self {
		self.cancel = label.into();
		self
	}
	/// Style the confirming action as destructive.
	pub fn danger(mut self) -> Self {
		self.tone = Tone::Danger;
		self.dialog = self.dialog.danger();
		self
	}
	/// Disable the confirming action, e.g. while a save is still in flight.
	pub fn enabled(mut self, enabled: bool) -> Self {
		self.enabled = enabled;
		self
	}
	/// Extra callout under the message.
	pub fn note(mut self, level: Level, text: impl Into<String>) -> Self {
		self.note = Some((level, text.into()));
		self
	}
	/// Returns the choice once the user makes one, otherwise `None`.
	pub fn show(self, ctx: &egui::Context) -> Option<Choice> {
		let Self {
			dialog,
			message,
			confirm,
			cancel,
			tone,
			enabled,
			note,
		} = self;
		let dialog_id = dialog.id;
		let mut choice = None;
		let response = dialog.show(ctx, |d| {
			d.content(|ui| {
				let colors = design::palette(ui);
				ui.add(
					egui::Label::new(RichText::new(&message).size(14.0).color(colors.text)).wrap(),
				);
				if let Some((level, text)) = &note {
					ui.add_space(12.0);
					notice(ui, *level, text);
				}
			});
			d.footer(|ui| {
				let kind = if tone == Tone::Danger {
					Action::Danger
				} else {
					Action::Primary
				};
				ui.add_enabled_ui(enabled, |ui| {
					if action(ui, &confirm, kind).clicked() {
						choice = Some(Choice::Confirmed);
					}
				});
				if action(ui, &cancel, Action::Neutral).clicked() {
					choice = Some(Choice::Cancelled);
				}
			});
		});
		if response.close {
			choice.get_or_insert(Choice::Cancelled);
		}
		enter_after_shown_frame(ctx, dialog_id, enabled, &mut choice);
		choice
	}
}

fn enter_after_shown_frame(
	ctx: &egui::Context,
	dialog_id: egui::Id,
	enabled: bool,
	choice: &mut Option<Choice>,
) {
	let key = dialog_id.with("shown-committed-frame");
	let frame = ctx.cumulative_frame_nr();
	let shown_committed_frame = ctx
		.data(|data| data.get_temp::<u64>(key))
		.is_some_and(|previous| previous + 1 == frame);
	if enabled
		&& shown_committed_frame
		&& choice.is_none()
		&& ctx.input_mut(|input| {
			let pressed = input.events.iter().any(|event| {
				matches!(
					event,
					egui::Event::Key {
						key: egui::Key::Enter,
						pressed: true,
						repeat: false,
						modifiers,
						..
					} if *modifiers == egui::Modifiers::NONE
				)
			});
			pressed && input.consume_key(egui::Modifiers::NONE, egui::Key::Enter)
		}) {
		*choice = Some(Choice::Confirmed);
	}
	if choice.is_some() {
		ctx.data_mut(|data| data.remove_temp::<u64>(key));
	} else if !ctx.will_discard() {
		ctx.data_mut(|data| data.insert_temp(key, frame));
	}
}
