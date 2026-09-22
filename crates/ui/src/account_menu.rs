//! Session-scoped account menu; the host owns presence publication.
use crate::{MessagingUi, design, icons, profiles};
use client_core::{Command, State};
use egui::{RichText, vec2};
use model::PresenceStatus;

#[derive(Default)]
pub(super) struct AccountMenu {
	open: bool,
	/// Set inside the popout: the popup owns `open` until its frame finishes.
	close: bool,
	generation: u64,
	draft: String,
	custom_open: bool,
	/// Chosen while the editor is open; only Apply commits it to a deadline.
	clear_after: ClearAfter,
}

/// Discord's own "Clear after" choices. The deadline is absolute once applied, so a status
/// set for an hour still clears an hour later even if the editor is reopened meanwhile.
#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(super) enum ClearAfter {
	#[default]
	Never,
	Minutes30,
	Hour,
	Hours4,
	Today,
}

impl ClearAfter {
	const ALL: [Self; 5] = [
		Self::Never,
		Self::Minutes30,
		Self::Hour,
		Self::Hours4,
		Self::Today,
	];
	fn label(self) -> &'static str {
		match self {
			Self::Never => "Don't clear",
			Self::Minutes30 => "30 minutes",
			Self::Hour => "1 hour",
			Self::Hours4 => "4 hours",
			Self::Today => "Today",
		}
	}
	/// Absolute deadline in milliseconds since the Unix epoch; `None` never clears.
	fn deadline(self) -> Option<u64> {
		let now = crate::local_time::now();
		let seconds = match self {
			Self::Never => return None,
			Self::Minutes30 => now.unix_timestamp() + 30 * 60,
			Self::Hour => now.unix_timestamp() + 60 * 60,
			Self::Hours4 => now.unix_timestamp() + 4 * 60 * 60,
			// End of the local day, which is what Discord means by "Today".
			Self::Today => {
				let midnight = now.replace_time(time::Time::MIDNIGHT);
				(midnight + time::Duration::days(1)).unix_timestamp()
			}
		};
		u64::try_from(seconds).ok().map(|seconds| seconds * 1000)
	}
	/// Plain-language moment this choice lands on, for the line under the dropdown.
	fn clears_at(self) -> Option<String> {
		if self == Self::Never {
			return None;
		}
		let now = crate::local_time::now();
		let at = match self {
			Self::Never => return None,
			Self::Minutes30 => now + time::Duration::minutes(30),
			Self::Hour => now + time::Duration::hours(1),
			Self::Hours4 => now + time::Duration::hours(4),
			Self::Today => now.replace_time(time::Time::MIDNIGHT) + time::Duration::days(1),
		};
		let clock = format!("{:02}:{:02}", at.hour(), at.minute());
		Some(if at.date() == now.date() {
			format!("at {clock}")
		} else {
			format!("at {clock} tomorrow")
		})
	}
	/// Nearest choice for an existing deadline, so reopening the editor shows what is set.
	fn nearest(expires: Option<u64>) -> Self {
		let Some(expires) = expires else {
			return Self::Never;
		};
		Self::ALL
			.into_iter()
			.skip(1)
			.find(|choice| {
				choice
					.deadline()
					.is_some_and(|deadline| expires <= deadline)
			})
			.unwrap_or(Self::Today)
	}
}

impl AccountMenu {
	/// Fixture-only: show the popout on the first frame of a demo capture.
	#[cfg(any(test, feature = "demo"))]
	pub(super) fn preview(&mut self, generation: u64) {
		self.open = true;
		self.generation = generation;
	}
	/// Fixture-only: open the custom-status editor over the popout.
	#[cfg(any(test, feature = "demo"))]
	pub(super) fn preview_editor(&mut self, generation: u64, draft: String) {
		self.preview(generation);
		self.clear_after = ClearAfter::Hour;
		self.draft = draft;
		self.custom_open = true;
	}
}

impl MessagingUi {
	pub fn adopt_account_presence(&mut self, presence: model::OwnPresence) {
		if !presence.custom_status.is_empty() && !presence.valid() {
			return;
		}
		if self.account_menu.draft == self.own_presence.custom_status {
			self.account_menu.draft.clone_from(&presence.custom_status);
			self.account_menu.clear_after = ClearAfter::nearest(presence.expires_at_ms);
		}
		self.own_presence_expires = presence.expires_at_ms;
		self.own_presence.expires_at_ms = presence.expires_at_ms;
		self.own_presence.status = presence.status;
		self.own_presence.custom_status = presence.custom_status;
	}

	pub(super) fn account_menu(
		&mut self,
		anchor: &egui::Response,
		state: &mut State,
		commands: &mut Vec<Command>,
	) {
		if self.account_menu.generation != state.generation {
			self.account_menu = AccountMenu {
				generation: state.generation,
				..Default::default()
			};
		}
		if anchor.clicked() {
			self.account_menu.open = !self.account_menu.open;
			if self.account_menu.open {
				self.account_menu.draft = self.own_presence.custom_status.clone();
				self.profile.hide();
				if state.own_profile.data.is_none()
					&& !state.own_profile.loading
					&& let Some(command) = state.load_own_profile()
				{
					commands.push(command);
				}
			}
		}
		let colors = design::palette_for(&anchor.ctx);
		let mut open = self.account_menu.open;
		egui::Popup::from_response(anchor)
			.id(anchor.id.with("account-menu"))
			.open_bool(&mut open)
			.align(egui::RectAlign::TOP_START)
			.close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
			.width(340.0)
			.frame(
				egui::Frame::popup(&anchor.ctx.style_of(anchor.ctx.theme()))
					.fill(colors.raised)
					.inner_margin(0)
					.corner_radius(10),
			)
			.show(|ui| {
				ui.set_width(340.0);
				let height = (ui.ctx().content_rect().height() - 90.0).clamp(180.0, 620.0);
				egui::ScrollArea::vertical()
					.min_scrolled_height(height)
					.max_height(height)
					.show(ui, |ui| self.account_menu_contents(ui, state, commands));
			});
		self.account_menu.open = open && !std::mem::take(&mut self.account_menu.close);
		if self.account_menu.custom_open {
			let ctx = anchor.ctx.clone();
			let response = crate::dialog::Dialog::new("custom-status-editor", "Custom status")
				.subtitle("Shown next to your name across Discord.")
				.width(420.0)
				.show(&ctx, |d| {
					d.content(|ui| self.custom_status_editor(ui, state));
				});
			if response.close {
				self.account_menu.custom_open = false;
			}
		}
	}

	fn account_menu_contents(
		&mut self,
		ui: &mut egui::Ui,
		state: &mut State,
		commands: &mut Vec<Command>,
	) {
		let colors = design::palette(ui);
		let (banner, _) =
			ui.allocate_exact_size(vec2(ui.available_width(), 64.0), egui::Sense::hover());
		let corner = egui::CornerRadius {
			nw: 10,
			ne: 10,
			sw: 0,
			se: 0,
		};
		if let Some(profile) = &state.own_profile.data {
			self.avatars
				.paint_banner(ui, profile, banner, corner, state.demo);
		} else {
			ui.painter().rect_filled(
				banner,
				corner,
				design::mix(colors.accent, colors.base, 0.55),
			);
		}
		if let Some(user) = &state.user {
			let avatar = egui::Rect::from_min_size(
				banner.left_bottom() + vec2(14.0, -38.0),
				vec2(72.0, 72.0),
			);
			// Thick popover-coloured ring, as Discord punches the avatar out of the banner.
			ui.painter()
				.circle_filled(avatar.center(), 42.0, colors.raised);
			ui.scope_builder(egui::UiBuilder::new().max_rect(avatar), |ui| {
				self.avatars.show(ui, user, 72.0, state.demo);
			});
			let dot = avatar.right_bottom() - egui::Vec2::splat(11.0);
			ui.painter().circle_filled(dot, 10.5, colors.raised);
			presence_dot(
				ui.painter(),
				dot,
				7.0,
				self.own_presence.status,
				colors.raised,
			);
			ui.add_space((avatar.bottom() + 12.0 - ui.cursor().top()).max(0.0));
		} else {
			ui.add_space(12.0);
		}
		egui::Frame::new()
			.inner_margin(egui::Margin {
				left: 12,
				right: 12,
				top: 0,
				bottom: 12,
			})
			.show(ui, |ui| {
				ui.set_width(ui.available_width());
				self.account_identity_card(ui, state, commands);
				if let Some(user) = &state.user {
					let guild = state
						.selected
						.and_then(|id| state.channel(id))
						.and_then(|c| c.guild);
					let (_, _, activities) = profiles::presence(state, user.id, guild);
					if !activities.is_empty() {
						ui.add_space(8.0);
						for activity in activities {
							profiles::activity_card(
								ui,
								activity,
								&mut self.avatars,
								state.demo,
								(colors.base, colors.muted),
							);
						}
					}
				}
				ui.add_space(8.0);
				ui.spacing_mut().item_spacing.y = 2.0;
				self.account_status_row(ui);
				self.account_custom_status_row(ui);
				self.account_switcher(ui, state);
			});
	}

	/// Other accounts remembered on this device, plus a row to remember one more.
	fn account_switcher(&mut self, ui: &mut egui::Ui, state: &State) {
		let colors = design::palette(ui);
		let current = state.user.as_ref().map(|user| user.id);
		// Bounded by MAX_SAVED_ACCOUNTS; cloned so the avatar cache stays mutably borrowable.
		let others: Vec<model::SavedAccount> = self
			.accounts
			.iter()
			.filter(|account| Some(account.id) != current)
			.cloned()
			.collect();
		ui.add_space(10.0);
		let (line, _) =
			ui.allocate_exact_size(vec2(ui.available_width(), 1.0), egui::Sense::hover());
		ui.painter().rect_filled(line, 0, colors.border);
		ui.add_space(10.0);
		ui.label(design::eyebrow(ui, "Switch accounts", colors.muted));
		ui.add_space(4.0);
		for account in &others {
			self.account_switcher_row(ui, account, state.demo);
		}
		let response = ui
			.scope(|ui| {
				let width = ui.available_width();
				ui.spacing_mut().button_padding = vec2(34.0, 10.0);
				ui.add(
					egui::Button::new(())
						.left_text(
							design::medium(ui, "Add an account", 14.0).color(colors.text_strong),
						)
						.frame_when_inactive(false)
						.corner_radius(6)
						.min_size(vec2(width, 40.0)),
				)
			})
			.inner;
		icons::paint(
			ui.painter(),
			icons::Icon::Plus,
			egui::Rect::from_center_size(
				egui::pos2(response.rect.left() + 17.0, response.rect.center().y),
				egui::Vec2::splat(17.0),
			),
			colors.muted,
		);
		if response.clicked() {
			self.add_account_requested = true;
			self.account_menu.close = true;
		}
	}

	/// One saved account: click to switch, trailing bin to forget it on this device.
	fn account_switcher_row(
		&mut self,
		ui: &mut egui::Ui,
		account: &model::SavedAccount,
		demo: bool,
	) {
		let colors = design::palette(ui);
		let (rect, _) =
			ui.allocate_exact_size(vec2(ui.available_width(), 44.0), egui::Sense::hover());
		let bin = egui::Rect::from_center_size(
			egui::pos2(rect.right() - 20.0, rect.center().y),
			egui::Vec2::splat(28.0),
		);
		let user = account.user();
		let avatar = egui::Rect::from_center_size(
			egui::pos2(rect.left() + 22.0, rect.center().y),
			egui::Vec2::splat(32.0),
		);
		let text = egui::Rect::from_min_max(
			egui::pos2(rect.left() + 48.0, rect.top() + 3.0),
			egui::pos2(bin.left() - 8.0, rect.bottom() - 3.0),
		);
		let over_bin = ui.rect_contains_pointer(bin);
		if ui.rect_contains_pointer(rect) {
			ui.painter().rect_filled(rect, 6, colors.hover);
		}
		ui.scope_builder(egui::UiBuilder::new().max_rect(avatar), |ui| {
			self.avatars.show_plain(ui, &user, 32.0, demo);
		});
		ui.scope_builder(egui::UiBuilder::new().max_rect(text), |ui| {
			ui.spacing_mut().item_spacing.y = 0.0;
			ui.add(
				egui::Label::new(
					design::medium(ui, account.label(), 14.0).color(colors.text_strong),
				)
				.truncate()
				.selectable(false),
			);
			ui.add(
				egui::Label::new(RichText::new(&account.name).size(12.0).color(colors.muted))
					.truncate()
					.selectable(false),
			);
		});
		// Same affordance as the sign-in screen's saved accounts.
		icons::paint(
			ui.painter(),
			icons::Icon::Close,
			bin.shrink(8.0),
			if over_bin {
				colors.danger
			} else {
				colors.muted
			},
		);
		// Registered after the contents so the row, not a label, receives the click.
		let row = ui.interact(
			rect,
			ui.scope_id().with(("switch-account", account.id.0)),
			egui::Sense::click(),
		);
		let forget = ui.interact(
			bin,
			ui.scope_id().with(("forget-account", account.id.0)),
			egui::Sense::click(),
		);
		if row.has_focus() || forget.has_focus() {
			ui.painter().rect_stroke(
				rect.shrink(1.0),
				6,
				egui::Stroke::new(1.0, colors.accent),
				egui::StrokeKind::Inside,
			);
		}
		row.widget_info(|| {
			egui::WidgetInfo::labeled(
				egui::Role::Button,
				true,
				format!("Switch to {}", account.label()),
			)
		});
		forget.widget_info(|| {
			egui::WidgetInfo::labeled(
				egui::Role::Button,
				true,
				format!("Forget {}", account.label()),
			)
		});
		let forget = forget.on_hover_text("Forget this account on this device");
		if forget.clicked() {
			self.forget_account_requested = Some(account.id);
			self.account_menu.close = true;
		} else if row.clicked() {
			self.switch_account_requested = Some(account.id);
			self.account_menu.close = true;
		}
	}

	/// Name, handle and current custom status, grouped on the sunken card Discord uses.
	fn account_identity_card(
		&mut self,
		ui: &mut egui::Ui,
		state: &mut State,
		commands: &mut Vec<Command>,
	) {
		let colors = design::palette(ui);
		egui::Frame::new()
			.fill(colors.base)
			.corner_radius(8)
			.inner_margin(12)
			.show(ui, |ui| {
				ui.set_width(ui.available_width());
				ui.spacing_mut().item_spacing.y = 2.0;
				let profile = state.own_profile.data.as_ref();
				let name = profile
					.and_then(|p| p.global_name.as_deref())
					.or_else(|| state.user.as_ref().map(|u| u.name.as_str()))
					.unwrap_or("Your account");
				ui.add(
					egui::Label::new(design::semibold(ui, name, 20.0).color(colors.text_strong))
						.wrap(),
				);
				if let Some(profile) = profile {
					ui.add(
						egui::Label::new(
							design::medium(ui, &profile.username, 14.0).color(colors.muted),
						)
						.wrap(),
					);
					if !profile.pronouns.is_empty() {
						ui.add(
							egui::Label::new(
								RichText::new(&profile.pronouns)
									.size(12.0)
									.color(colors.muted),
							)
							.wrap(),
						);
					}
				}
				if !self.own_presence.custom_status.is_empty() {
					ui.add_space(10.0);
					let (line, _) = ui
						.allocate_exact_size(vec2(ui.available_width(), 1.0), egui::Sense::hover());
					ui.painter().rect_filled(line, 0, colors.border);
					ui.add_space(10.0);
					ui.add(
						egui::Label::new(
							RichText::new(&self.own_presence.custom_status)
								.size(14.0)
								.color(colors.text),
						)
						.wrap(),
					);
				}
				if state.own_profile.loading {
					ui.add_space(6.0);
					ui.label(
						RichText::new("Loading profile…")
							.small()
							.color(colors.muted),
					);
				}
				if let Some(error) = state.own_profile.error {
					ui.add_space(6.0);
					ui.add(
						egui::Label::new(RichText::new(error).size(12.0).color(colors.danger))
							.wrap(),
					);
					if ui.button("Reload profile").clicked()
						&& let Some(command) = state.load_own_profile()
					{
						commands.push(command);
					}
				}
			});
	}

	/// Presence row: status dot, label and a chevron opening the status menu.
	fn account_status_row(&mut self, ui: &mut egui::Ui) {
		let colors = design::palette(ui);
		let status = self.own_presence.status;
		let label = design::medium(ui, status.label(), 14.0).color(colors.text_strong);
		let response = ui
			.scope(|ui| {
				let width = ui.available_width();
				ui.spacing_mut().button_padding = vec2(34.0, 10.0);
				egui::containers::menu::MenuButton::from_button(
					egui::Button::new(())
						.left_text(label)
						.frame_when_inactive(false)
						.corner_radius(6)
						.min_size(vec2(width, 40.0)),
				)
				.ui(ui, |ui| self.presence_menu(ui))
				.0
			})
			.inner;
		let rect = response.rect;
		let background = if response.hovered() || response.has_focus() {
			ui.visuals().widgets.hovered.weak_bg_fill
		} else {
			colors.raised
		};
		presence_dot(
			ui.painter(),
			egui::pos2(rect.left() + 17.0, rect.center().y),
			5.0,
			status,
			background,
		);
		icons::paint(
			ui.painter(),
			icons::Icon::ChevronRight,
			egui::Rect::from_center_size(
				egui::pos2(rect.right() - 18.0, rect.center().y),
				egui::Vec2::splat(16.0),
			),
			colors.muted,
		);
	}

	fn presence_menu(&mut self, ui: &mut egui::Ui) {
		let colors = design::palette(ui);
		ui.set_width(260.0_f32.min(ui.ctx().content_rect().width() - 48.0));
		for status in PresenceStatus::ALL {
			let description = match status {
				PresenceStatus::DoNotDisturb => "You will not receive desktop notifications",
				PresenceStatus::Invisible => "You will appear offline",
				_ => "",
			};
			let height = if description.is_empty() { 40.0 } else { 62.0 };
			let response = ui.add_sized(
				[ui.available_width(), height],
				egui::Button::new("")
					.frame_when_inactive(false)
					.corner_radius(6),
			);
			response.widget_info(|| {
				egui::WidgetInfo::labeled(egui::Role::Button, true, status.label())
			});
			let x = response.rect.left() + 34.0;
			let y = response.rect.top() + if description.is_empty() { 11.0 } else { 10.0 };
			let background = if response.hovered() || response.has_focus() {
				ui.visuals().widgets.hovered.weak_bg_fill
			} else {
				ui.visuals().window_fill
			};
			presence_dot(
				ui.painter(),
				egui::pos2(response.rect.left() + 17.0, response.rect.center().y),
				5.0,
				status,
				background,
			);
			ui.painter().text(
				egui::pos2(x, y),
				egui::Align2::LEFT_TOP,
				status.label(),
				egui::FontId::new(14.0, design::medium_family(ui.ctx())),
				colors.text_strong,
			);
			if !description.is_empty() {
				let galley = ui.painter().layout(
					description.into(),
					egui::FontId::proportional(12.0),
					colors.muted,
					(response.rect.width() - 46.0).max(80.0),
				);
				ui.painter()
					.galley(egui::pos2(x, y + 22.0), galley, colors.muted);
			}
			if response.clicked() {
				if self.own_presence.status != status {
					self.own_presence.status = status;
					self.own_presence_changed = true;
				}
				ui.close();
			}
			if status == PresenceStatus::Online {
				ui.separator();
			}
		}
	}

	/// Custom status row: smiley to write one, pencil once a status is set.
	fn account_custom_status_row(&mut self, ui: &mut egui::Ui) {
		let colors = design::palette(ui);
		let set = !self.own_presence.custom_status.is_empty();
		let label = if set {
			"Edit custom status"
		} else {
			"Set a custom status"
		};
		let text = design::medium(ui, label, 14.0).color(colors.text_strong);
		let response = ui
			.scope(|ui| {
				let width = ui.available_width();
				ui.spacing_mut().button_padding = vec2(34.0, 10.0);
				ui.add(
					egui::Button::new(())
						.left_text(text)
						.frame_when_inactive(false)
						.corner_radius(6)
						.min_size(vec2(width, 40.0)),
				)
			})
			.inner;
		icons::paint(
			ui.painter(),
			if set {
				icons::Icon::Pencil
			} else {
				icons::Icon::Smile
			},
			egui::Rect::from_center_size(
				egui::pos2(response.rect.left() + 17.0, response.rect.center().y),
				egui::Vec2::splat(17.0),
			),
			colors.muted,
		);
		if response.clicked() {
			self.account_menu
				.draft
				.clone_from(&self.own_presence.custom_status);
			self.account_menu.clear_after = ClearAfter::nearest(self.own_presence_expires);
			self.account_menu.custom_open = true;
		}
	}
	/// "Clear after" dropdown, matching the presence rows: value on the left, chevron right.
	fn clear_after_row(&mut self, ui: &mut egui::Ui) -> egui::Response {
		let colors = design::palette(ui);
		let chosen = self.account_menu.clear_after;
		let label = design::medium(ui, chosen.label(), 14.0).color(colors.text_strong);
		let response = ui
			.scope(|ui| {
				let width = ui.available_width();
				ui.spacing_mut().button_padding = vec2(14.0, 10.0);
				egui::containers::menu::MenuButton::from_button(
					egui::Button::new(())
						.left_text(label)
						.frame_when_inactive(false)
						.corner_radius(8)
						.min_size(vec2(width, 44.0)),
				)
				.ui(ui, |ui| {
					ui.set_width(220.0_f32.min(ui.ctx().content_rect().width() - 48.0));
					for choice in ClearAfter::ALL {
						let picked = choice == chosen;
						let response = ui.add_sized(
							[ui.available_width(), 36.0],
							egui::Button::new(())
								.left_text(
									design::medium(ui, choice.label(), 14.0)
										.color(colors.text_strong),
								)
								.frame_when_inactive(picked)
								.corner_radius(6),
						);
						if response.clicked() {
							self.account_menu.clear_after = choice;
							ui.close();
						}
					}
				})
				.0
			})
			.inner;
		let rect = response.rect;
		ui.painter().rect_stroke(
			rect,
			8,
			egui::Stroke::new(1.0, colors.border),
			egui::StrokeKind::Inside,
		);
		icons::paint(
			ui.painter(),
			icons::Icon::ChevronDown,
			egui::Rect::from_center_size(
				egui::pos2(rect.right() - 18.0, rect.center().y),
				egui::Vec2::splat(14.0),
			),
			colors.muted,
		);
		response
	}

	/// Live preview, bounded field and footer actions, styled like Discord's dialog.
	fn custom_status_editor(&mut self, ui: &mut egui::Ui, state: &State) {
		let colors = design::palette(ui);
		let draft = self.account_menu.draft.trim().to_owned();
		egui::Frame::new()
			.fill(colors.base)
			.corner_radius(8)
			.inner_margin(12)
			.show(ui, |ui| {
				ui.set_width(ui.available_width());
				ui.horizontal(|ui| {
					ui.spacing_mut().item_spacing.x = 10.0;
					let name = if let Some(user) = &state.user {
						self.avatars.show(ui, user, 40.0, state.demo);
						state
							.own_profile
							.data
							.as_ref()
							.and_then(|p| p.global_name.as_deref())
							.unwrap_or(user.name.as_str())
					} else {
						design::avatar(ui, "You", 40.0);
						"Your account"
					};
					let width = ui.available_width();
					ui.vertical(|ui| {
						ui.set_width(width);
						ui.spacing_mut().item_spacing.y = 2.0;
						ui.add(
							egui::Label::new(
								design::semibold(ui, name, 15.0).color(colors.text_strong),
							)
							.truncate(),
						);
						let (status, color) = if draft.is_empty() {
							("No custom status", colors.muted)
						} else {
							(draft.as_str(), colors.text)
						};
						ui.add(
							egui::Label::new(RichText::new(status).size(13.0).color(color))
								.truncate(),
						);
					});
				});
			});
		ui.add_space(16.0);
		let label = ui.label(design::eyebrow(ui, "Status text", colors.muted));
		ui.add_space(6.0);
		ui.add_sized(
			[ui.available_width(), 44.0],
			egui::TextEdit::singleline(&mut self.account_menu.draft)
				.align(egui::Align2::LEFT_CENTER)
				.id_salt(("account-custom-status", state.generation))
				.hint_text("What's on your mind?")
				.char_limit(128)
				.margin(egui::Margin::symmetric(12, 0))
				.desired_width(f32::INFINITY),
		)
		.labelled_by(label.id);
		let valid = model::OwnPresence {
			status: self.own_presence.status,
			custom_status: draft.clone(),
			expires_at_ms: None,
		}
		.valid();
		let changed = draft != self.own_presence.custom_status
			|| (!draft.is_empty()
				&& ClearAfter::nearest(self.own_presence_expires) != self.account_menu.clear_after);
		ui.add_space(4.0);
		// A bounded row: a bare right-to-left layout here takes the dialog's whole remaining
		// height and the size never settles.
		ui.horizontal(|ui| {
			ui.set_height(14.0);
			ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
				let used = self.account_menu.draft.chars().count();
				ui.label(
					RichText::new(format!("{used}/128"))
						.size(12.0)
						.color(if used > 112 {
							colors.danger
						} else {
							colors.muted
						}),
				);
			});
		});
		ui.add_space(12.0);
		let label = ui.label(design::eyebrow(ui, "Clear after", colors.muted));
		ui.add_space(6.0);
		self.clear_after_row(ui).labelled_by(label.id);
		// The deadline is local to this client, so name the moment rather than implying
		// Discord will clear it for you.
		if let Some(clears) = self.account_menu.clear_after.clears_at() {
			ui.add_space(6.0);
			ui.add(
				egui::Label::new(
					RichText::new(format!("Serein clears it {clears}."))
						.size(12.0)
						.color(colors.muted),
				)
				.wrap(),
			);
		}
		if !valid {
			ui.add_space(4.0);
			ui.add(
				egui::Label::new(
					RichText::new("Use up to 128 characters without control characters.")
						.size(12.0)
						.color(colors.danger),
				)
				.wrap(),
			);
		}
		if !self.own_presence_status.is_empty() {
			ui.add_space(4.0);
			ui.add(
				egui::Label::new(
					RichText::new(self.own_presence_status)
						.size(12.0)
						.color(colors.muted),
				)
				.wrap(),
			);
		}
		ui.add_space(16.0);
		let clearable =
			!self.own_presence.custom_status.is_empty() || !self.account_menu.draft.is_empty();
		let width = ui.available_width();
		ui.horizontal(|ui| {
			ui.spacing_mut().item_spacing.x = 8.0;
			let half = (width - 8.0) * 0.5;
			ui.allocate_ui(vec2(half, 44.0), |ui| {
				ui.set_width(half);
				if ui
					.add_enabled_ui(clearable, |ui| design::secondary_button(ui, "Clear"))
					.inner
					.clicked()
				{
					self.account_menu.draft.clear();
					self.account_menu.clear_after = ClearAfter::Never;
					self.own_presence_expires = None;
					self.own_presence.expires_at_ms = None;
					if !self.own_presence.custom_status.is_empty() {
						self.own_presence.custom_status.clear();
						self.own_presence_changed = true;
					}
				}
			});
			ui.allocate_ui(vec2(half, 44.0), |ui| {
				ui.set_width(half);
				if ui
					.add_enabled_ui(valid && changed, |ui| design::primary_button(ui, "Apply"))
					.inner
					.clicked()
				{
					self.own_presence.custom_status = draft.clone();
					self.account_menu
						.draft
						.clone_from(&self.own_presence.custom_status);
					self.own_presence_expires = (!draft.is_empty())
						.then(|| self.account_menu.clear_after.deadline())
						.flatten();
					self.own_presence.expires_at_ms = self.own_presence_expires;
					self.own_presence_changed = true;
				}
			});
		});
	}
}

/// Presence dot with Discord's status glyphs punched out in the surface colour.
fn presence_dot(
	painter: &egui::Painter,
	center: egui::Pos2,
	radius: f32,
	status: PresenceStatus,
	background: egui::Color32,
) {
	painter.circle_filled(center, radius, profiles::presence_color(status.wire()));
	match status {
		PresenceStatus::Idle => {
			painter.circle_filled(
				center - egui::Vec2::splat(radius * 0.5),
				radius * 0.8,
				background,
			);
		}
		PresenceStatus::Invisible => {
			painter.circle_filled(center, radius * 0.56, background);
		}
		PresenceStatus::DoNotDisturb => {
			painter.line_segment(
				[
					center - vec2(radius * 0.62, 0.0),
					center + vec2(radius * 0.62, 0.0),
				],
				egui::Stroke::new(radius * 0.42, background),
			);
		}
		_ => {}
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use egui::{Event, Pos2, Rect};

	fn frame(
		ctx: &egui::Context,
		view: &mut MessagingUi,
		state: &mut State,
		size: egui::Vec2,
		events: Vec<Event>,
	) -> Vec<(String, Rect)> {
		let mut text = vec![];
		let output = ctx.run_ui(
			egui::RawInput {
				screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)),
				events,
				..Default::default()
			},
			|ui| {
				egui::Panel::bottom("test-account-footer").show(ui, |ui| {
					let mut commands = vec![];
					view.account_card(ui, state, &mut commands);
					assert!(commands.is_empty());
				});
			},
		);
		fn collect(shape: &egui::Shape, labels: &mut Vec<(String, Rect)>) {
			match shape {
				egui::Shape::Text(t) => labels.push((
					t.galley.job.text.clone(),
					Rect::from_min_size(t.pos, t.galley.size()),
				)),
				egui::Shape::Vec(shapes) => {
					for shape in shapes {
						collect(shape, labels);
					}
				}
				_ => {}
			}
		}
		for shape in &output.shapes {
			collect(&shape.shape, &mut text);
		}
		assert!(output.platform_output.commands.is_empty());
		output.drop_without_applying_deltas();
		text
	}
	fn click(
		ctx: &egui::Context,
		view: &mut MessagingUi,
		state: &mut State,
		size: egui::Vec2,
		position: Pos2,
	) {
		for pressed in [true, false] {
			frame(
				ctx,
				view,
				state,
				size,
				vec![
					Event::PointerMoved(position),
					Event::PointerButton {
						pos: position,
						button: egui::PointerButton::Primary,
						pressed,
						modifiers: egui::Modifiers::NONE,
					},
				],
			);
		}
	}
	fn locate(text: &[(String, Rect)], label: &str) -> Pos2 {
		text.iter()
			.find(|(text, _)| text == label)
			.unwrap_or_else(|| panic!("Missing {label}: {text:?}"))
			.1
			.center()
	}
	fn alt_account() -> model::SavedAccount {
		model::SavedAccount {
			id: model::Id(424_242),
			name: "synthetic-alt".into(),
			display: Some("Synthetic Alt".into()),
			avatar: None,
			discriminator: 0,
			has_token: true,
		}
	}

	#[test]
	fn switcher_lists_other_accounts_without_the_signed_in_one() {
		let size = vec2(340.0, 900.0);
		for demo in [true, false] {
			let ctx = egui::Context::default();
			design::apply(&ctx);
			let mut state = test_support::demo_state();
			state.demo = demo;
			let own = state.user.as_ref().unwrap().clone();
			state.own_profile.data = Some(profiles::synthetic(&own, None));
			// The signed-in account is remembered too, and never offered as a switch target.
			let mut view = MessagingUi {
				accounts: vec![
					model::SavedAccount {
						id: own.id,
						name: own.name.clone(),
						display: Some("Signed in already".into()),
						avatar: None,
						discriminator: own.discriminator,
						has_token: true,
					},
					alt_account(),
				],
				..Default::default()
			};
			view.preview_account_menu(state.generation);
			for _ in 0..3 {
				frame(&ctx, &mut view, &mut state, size, vec![]);
			}
			let text = frame(&ctx, &mut view, &mut state, size, vec![]);
			let listed = |label: &str| text.iter().any(|(value, _)| value == label);
			assert!(listed("SWITCH ACCOUNTS"));
			assert!(listed("Synthetic Alt"));
			assert!(listed("synthetic-alt"));
			assert!(listed("Add an account"));
			// The signed-in account is remembered but never listed as a switch target.
			assert!(!listed("Signed in already"));
			click(
				&ctx,
				&mut view,
				&mut state,
				size,
				locate(&text, "Add an account"),
			);
			assert!(view.add_account_requested);
			assert!(!view.account_menu.open);
			assert_eq!(view.switch_account_requested, None);
		}
	}

	#[test]
	fn switcher_row_separates_switching_from_forgetting() {
		let account = alt_account();
		let row = |view: &mut MessagingUi, ctx: &egui::Context, events: Vec<Event>| {
			let output = ctx.run_ui(
				egui::RawInput {
					screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(300.0, 120.0))),
					events,
					..Default::default()
				},
				|ui| {
					ui.scope_builder(
						egui::UiBuilder::new()
							.max_rect(Rect::from_min_size(Pos2::ZERO, vec2(300.0, 44.0))),
						|ui| view.account_switcher_row(ui, &account, false),
					);
				},
			);
			output.drop_without_applying_deltas();
		};
		// The row spans the full width; the trailing bin owns only its own corner.
		for (position, switches) in [
			(Pos2::new(120.0, 22.0), true),
			(Pos2::new(280.0, 22.0), false),
		] {
			let ctx = egui::Context::default();
			design::apply(&ctx);
			let mut view = MessagingUi::default();
			// The pointer lands on widgets registered by an earlier frame.
			row(&mut view, &ctx, vec![]);
			for pressed in [true, false] {
				row(
					&mut view,
					&ctx,
					vec![
						Event::PointerMoved(position),
						Event::PointerButton {
							pos: position,
							button: egui::PointerButton::Primary,
							pressed,
							modifiers: egui::Modifiers::NONE,
						},
					],
				);
			}
			assert_eq!(
				view.switch_account_requested,
				switches.then_some(account.id)
			);
			assert_eq!(
				view.forget_account_requested,
				(!switches).then_some(account.id)
			);
			// The popout owns `open`; rows only ask it to close.
			assert!(view.account_menu.close);
		}
	}

	#[test]
	fn account_menu_opens_applies_clears_and_closes_across_themes_and_sizes() {
		for light in [false, true] {
			for size in [vec2(340.0, 760.0), vec2(760.0, 520.0)] {
				let ctx = egui::Context::default();
				ctx.set_theme(if light {
					egui::ThemePreference::Light
				} else {
					egui::ThemePreference::Dark
				});
				design::apply(&ctx);
				let mut state = test_support::demo_state();
				let own = state.user.as_ref().unwrap().clone();
				state.own_profile.data = Some(profiles::synthetic(&own, None));
				let mut view = MessagingUi::default();
				frame(&ctx, &mut view, &mut state, size, vec![]);
				let text = frame(&ctx, &mut view, &mut state, size, vec![]);
				if light {
					for key in [egui::Key::Tab, egui::Key::Enter] {
						frame(
							&ctx,
							&mut view,
							&mut state,
							size,
							vec![Event::Key {
								key,
								physical_key: None,
								pressed: true,
								repeat: false,
								modifiers: egui::Modifiers::NONE,
							}],
						);
					}
				} else {
					click(&ctx, &mut view, &mut state, size, locate(&text, &own.name));
				}
				for _ in 0..3 {
					frame(&ctx, &mut view, &mut state, size, vec![]);
				}
				assert!(view.account_menu.open);
				let text = frame(&ctx, &mut view, &mut state, size, vec![]);
				click(&ctx, &mut view, &mut state, size, locate(&text, "Online"));
				for _ in 0..3 {
					frame(&ctx, &mut view, &mut state, size, vec![]);
				}
				let text = frame(&ctx, &mut view, &mut state, size, vec![]);
				for status in PresenceStatus::ALL {
					let p = locate(&text, status.label());
					assert!(Rect::from_min_size(Pos2::ZERO, size).contains(p));
				}
				click(
					&ctx,
					&mut view,
					&mut state,
					size,
					locate(&text, "Invisible"),
				);
				assert_eq!(view.own_presence.status, PresenceStatus::Invisible);
				assert!(std::mem::take(&mut view.own_presence_changed));
				view.account_menu.open = true;
				for _ in 0..3 {
					frame(&ctx, &mut view, &mut state, size, vec![]);
				}
				let text = frame(&ctx, &mut view, &mut state, size, vec![]);
				click(
					&ctx,
					&mut view,
					&mut state,
					size,
					locate(&text, "Set a custom status"),
				);
				assert!(view.account_menu.custom_open);
				// A bounded draft is separate from the value published by the host.
				view.account_menu.draft = "  Synthetic status 🌙  ".into();
				assert!(view.own_presence.custom_status.is_empty());
				// Tall viewport keeps the editor in view; narrow/short mode additionally checks scrolling above.
				let size = vec2(size.x, 900.0);
				for _ in 0..3 {
					frame(&ctx, &mut view, &mut state, size, vec![]);
				}
				let text = frame(&ctx, &mut view, &mut state, size, vec![]);
				click(&ctx, &mut view, &mut state, size, locate(&text, "Apply"));
				assert_eq!(view.own_presence.custom_status, "Synthetic status 🌙");
				assert!(std::mem::take(&mut view.own_presence_changed));
				let text = frame(&ctx, &mut view, &mut state, size, vec![]);
				click(&ctx, &mut view, &mut state, size, locate(&text, "Clear"));
				assert!(view.own_presence.custom_status.is_empty());
				assert!(view.own_presence_changed);
				frame(
					&ctx,
					&mut view,
					&mut state,
					size,
					vec![Event::Key {
						key: egui::Key::Escape,
						physical_key: None,
						pressed: true,
						repeat: false,
						modifiers: egui::Modifiers::NONE,
					}],
				);
				assert!(!view.account_menu.custom_open);
				view.account_menu.open = true;
				view.account_menu.draft = "Old account draft".into();
				state.generation += 1;
				frame(&ctx, &mut view, &mut state, size, vec![]);
				assert!(!view.account_menu.open && view.account_menu.draft.is_empty());
				assert!(view.take_avatar_requests().is_empty());
			}
		}
	}
}
