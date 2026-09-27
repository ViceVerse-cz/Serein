//! Read-only audit events, filtered on demand and rendered only near the viewport.
use crate::{avatars::Avatars, design, icons};
use client_core::{Command, State};
use egui::{RichText, Vec2};
use model::{
	Id, Patch, server_admin,
	server_audit_log::{Change, Entry, Page, Query},
};

#[derive(Default)]
pub(super) struct AuditLogUi {
	query: Query,
	expanded: Option<Id>,
	preview_expand: bool,
}
impl AuditLogUi {
	pub fn preview(&mut self, expanded: bool) {
		self.preview_expand = expanded;
	}
	pub fn load(&mut self, state: &mut State, guild: Id) -> Option<Command> {
		if state.server_admin.pending
			|| state.server_admin.error.is_some()
			|| (state.server_admin.guild == Some(guild) && state.server_admin.audit_log.is_some())
		{
			return None;
		}
		state.request_server_admin(guild, server_admin::Action::AuditLog(self.query.clone()))
	}
	pub fn show(
		&mut self,
		ui: &mut egui::Ui,
		state: &mut State,
		guild: Id,
		avatars: &mut Avatars,
		commands: &mut Vec<Command>,
	) {
		let available = !state.server_admin.pending && !state.server_settings.saving;
		let mut query = self.query.clone();
		query.before = None;
		let width = ui.available_width();

		let wide = width >= 580.0;
		let picker_width = if wide {
			(width * 0.32).min(240.0)
		} else {
			(width - 12.0) / 2.0
		};
		let (header, _) = ui.allocate_exact_size(
			Vec2::new(width, if wide { 58.0 } else { 104.0 }),
			egui::Sense::hover(),
		);
		let heading_width = if wide {
			width - picker_width * 2.0 - 24.0
		} else {
			width
		};
		let mut heading = ui.new_child(
			egui::UiBuilder::new()
				.id_salt("audit-heading")
				.max_rect(egui::Rect::from_min_size(
					header.min,
					Vec2::new(heading_width, if wide { 58.0 } else { 36.0 }),
				))
				.layout(egui::Layout::left_to_right(egui::Align::Center)),
		);
		heading.label(design::semibold(
			&heading,
			crate::i18n::translate("server-audit-log-show-audit-log"),
			20.0,
		));
		let filter_start = header.min
			+ if wide {
				Vec2::new(heading_width + 12.0, 0.0)
			} else {
				Vec2::new(0.0, 46.0)
			};
		{
			let mut child = ui.new_child(
				egui::UiBuilder::new()
					.id_salt("audit-user-filter")
					.max_rect(egui::Rect::from_min_size(
						filter_start,
						Vec2::new(picker_width, 58.0),
					)),
			);
			let ui = &mut child;
			if !available {
				ui.disable();
			}
			ui.label(design::medium(
				ui,
				crate::i18n::translate("server-audit-log-show-filter-by-user"),
				14.0,
			));
			let selected = query
				.user
				.map(|id| user_name(state.server_admin.audit_log.as_ref(), id))
				.unwrap_or_else(|| crate::i18n::translate("server-audit-log-show-all-users"));
			egui::ComboBox::from_id_salt("audit-user")
				.selected_text(selected)
				.width(picker_width)
				.truncate()
				.show_ui(ui, |ui| {
					ui.selectable_value(
						&mut query.user,
						None,
						crate::i18n::translate("server-audit-log-show-all-users"),
					);
					if let Some(page) = &state.server_admin.audit_log {
						for user in page.users.iter().filter(|user| {
							page.entries
								.iter()
								.any(|entry| entry.user_id == Some(user.id))
						}) {
							ui.selectable_value(&mut query.user, Some(user.id), &user.name);
						}
					}
				});
		}
		{
			let mut child = ui.new_child(
				egui::UiBuilder::new()
					.id_salt("audit-action-filter")
					.max_rect(egui::Rect::from_min_size(
						filter_start + Vec2::new(picker_width + 12.0, 0.0),
						Vec2::new(picker_width, 58.0),
					)),
			);
			let ui = &mut child;
			if !available {
				ui.disable();
			}
			ui.label(design::medium(
				ui,
				crate::i18n::translate("server-audit-log-show-filter-by-action"),
				14.0,
			));
			let selected = query
				.action
				.map(|action| crate::i18n::translate_if_key(action_text(action)))
				.unwrap_or_else(|| crate::i18n::translate("server-audit-log-show-all-actions"));
			egui::ComboBox::from_id_salt("audit-action")
				.selected_text(selected)
				.width(picker_width)
				.truncate()
				.show_ui(ui, |ui| {
					ui.selectable_value(
						&mut query.action,
						None,
						crate::i18n::translate("server-audit-log-show-all-actions"),
					);
					for &(action, label) in ACTIONS {
						ui.selectable_value(
							&mut query.action,
							Some(action),
							crate::i18n::translate_if_key(label),
						);
					}
				});
		}
		ui.add_space(16.0);
		ui.separator();
		ui.add_space(16.0);
		let mut requested = if query != self.query {
			Some(query.clone())
		} else {
			None
		};
		ui.horizontal(|ui| {
			if ui
				.add_enabled(
					available,
					egui::Button::new(crate::i18n::translate("server-audit-log-show-reload"))
						.frame(false),
				)
				.clicked()
			{
				requested = Some(query.clone());
			}
			if state.server_admin.pending {
				ui.spinner();
				ui.weak(crate::i18n::translate(
					"server-audit-log-show-loading-audit-log",
				));
			}
		});
		if let Some(error) = state.server_admin.error {
			crate::dialog::notice(ui, crate::dialog::Level::Error, error);
		}
		if let Some(page) = &state.server_admin.audit_log {
			if self.preview_expand {
				self.expanded = page
					.entries
					.iter()
					.find(|entry| entry.action_type == 40 && !entry.changes.is_empty())
					.map(|entry| entry.id);
				self.preview_expand = false;
			}
			if page.entries.is_empty() && !state.server_admin.pending {
				ui.add_space(24.0);
				ui.weak(crate::i18n::translate(
					"server-audit-log-show-no-audit-log-entries-match-these-filters",
				));
			}
			// The list is the page's only scroller: it takes the remaining height and keeps
			// the paging controls pinned below it.
			let paging = state.server_admin.audit_limit_reached || page.has_more;
			let height = design::list_height(ui, if paging { 60.0 } else { 0.0 });
			self.entries(ui, state, page, avatars, height);
			if paging {
				ui.add_space(12.0);
			}
			if state.server_admin.audit_limit_reached {
				ui.weak(crate::i18n::translate(
					"server-audit-log-show-the-audit-log-reached-its-local-entry-or-memory-limit",
				));
			} else if page.has_more
				&& let Some(last) = page.entries.last()
				&& ui
					.add_enabled(
						available,
						egui::Button::new(crate::i18n::translate(
							"server-audit-log-show-load-more",
						))
						.min_size(Vec2::new(120.0, 36.0)),
					)
					.clicked()
			{
				query.before = Some(last.id);
				requested = Some(query);
			}
		}
		if let Some(query) = requested
			&& let Some(command) =
				state.request_server_admin(guild, server_admin::Action::AuditLog(query.clone()))
		{
			commands.push(command);
			if query.before.is_none() {
				self.expanded = None;
			}
			self.query = Query {
				before: None,
				..query
			};
		}
	}
	fn entries(
		&mut self,
		ui: &mut egui::Ui,
		state: &State,
		page: &Page,
		avatars: &mut Avatars,
		height: f32,
	) {
		let expanded_index = self
			.expanded
			.and_then(|id| page.entries.iter().position(|entry| entry.id == id));
		let extra = expanded_index.map_or(0.0, |i| details_height(&page.entries[i]));
		let width = ui.available_width();
		let total_height = page.entries.len() as f32 * 80.0 + extra;
		egui::ScrollArea::vertical()
			.id_salt("audit-log-entries")
			.max_height(height)
			.auto_shrink([false, true])
			.show_viewport(ui, |ui, viewport| {
				ui.set_min_height(total_height);
				let origin = ui.cursor().min;
				let first = ((viewport.min.y - extra).max(0.0) / 80.0).floor() as usize;
				let end = ((viewport.max.y / 80.0).ceil() as usize + 1).min(page.entries.len());
				for index in first..end {
					let entry = &page.entries[index];
					let expanded = expanded_index == Some(index);
					let y = index as f32 * 80.0
						+ if expanded_index.is_some_and(|open| index > open) {
							extra
						} else {
							0.0
						};
					let rect = egui::Rect::from_min_size(
						origin + Vec2::new(0.0, y),
						Vec2::new(width, 72.0 + if expanded { extra } else { 0.0 }),
					);
					if !ui.is_rect_visible(rect) {
						continue;
					}
					let colors = design::palette(ui);
					ui.painter().rect_filled(rect, 8, colors.raised);
					ui.painter().rect_stroke(
						rect,
						8,
						egui::Stroke::new(1.0, colors.border),
						egui::StrokeKind::Inside,
					);
					let header = egui::Rect::from_min_size(
						rect.min + Vec2::splat(4.0),
						Vec2::new(rect.width() - 8.0, 64.0),
					);
					let response = ui.interact(
						header,
						ui.scope_id().with(("audit-entry", entry.id)),
						egui::Sense::click(),
					);
					let summary = summary(entry, page, state);
					response.widget_info(|| {
						egui::WidgetInfo::selected(
							egui::Role::Button,
							ui.is_enabled(),
							expanded,
							&summary,
						)
					});
					if response.hovered() || response.has_focus() || expanded {
						ui.painter().rect_filled(header, 5, colors.selected);
					}
					if response.clicked() {
						self.expanded = if expanded { None } else { Some(entry.id) };
					}
					response.on_hover_text(&summary);
					let kind_rect = egui::Rect::from_center_size(
						egui::pos2(header.left() + 22.0, header.center().y),
						Vec2::splat(22.0),
					);
					icons::paint(
						ui.painter(),
						event_icon(entry.action_type),
						kind_rect,
						colors.muted,
					);
					let avatar = egui::Rect::from_center_size(
						egui::pos2(header.left() + 60.0, header.center().y),
						Vec2::splat(36.0),
					);
					{
						let mut child = ui.new_child(
							egui::UiBuilder::new()
								.id_salt(("audit-avatar", entry.id))
								.max_rect(avatar),
						);
						let ui = &mut child;

						if let Some(user) = entry
							.user_id
							.and_then(|id| page.users.iter().find(|user| user.id == id))
						{
							avatars.show(ui, user, 36.0, state.demo);
						} else {
							icons::paint(ui.painter(), icons::Icon::Profile, avatar, colors.muted);
						}
					}
					let text_rect = egui::Rect::from_min_max(
						header.min + Vec2::new(88.0, 12.0),
						header.max - Vec2::new(36.0, 8.0),
					);
					{
						let mut child = ui.new_child(
							egui::UiBuilder::new()
								.id_salt(("audit-text", entry.id))
								.max_rect(text_rect),
						);
						let ui = &mut child;

						ui.spacing_mut().item_spacing.y = 1.0;
						ui.add(egui::Label::new(design::medium(ui, summary, 14.0)).truncate());
						ui.add(
							egui::Label::new(
								RichText::new(timestamp(entry.id))
									.color(colors.muted)
									.size(12.0),
							)
							.truncate(),
						);
					}
					icons::paint(
						ui.painter(),
						if expanded {
							icons::Icon::ChevronDown
						} else {
							icons::Icon::ChevronRight
						},
						egui::Rect::from_center_size(
							egui::pos2(header.right() - 22.0, header.center().y),
							Vec2::splat(18.0),
						),
						colors.muted,
					);
					if expanded {
						let body = egui::Rect::from_min_max(
							rect.min + Vec2::new(24.0, 78.0),
							rect.max - Vec2::new(16.0, 12.0),
						);
						{
							let mut child = ui.new_child(
								egui::UiBuilder::new()
									.id_salt(("audit-details", entry.id))
									.max_rect(body),
							);
							let ui = &mut child;

							egui::ScrollArea::vertical()
								.id_salt(("audit-details", entry.id))
								.max_height(body.height())
								.auto_shrink([false, true])
								.show(ui, |ui| {
									details(ui, entry, state);
								});
						}
					}
				}
			});
	}
}
fn details_height(entry: &Entry) -> f32 {
	((entry.changes.len() + entry.options.len() + usize::from(entry.reason.is_some())).max(1)
		as f32 * 28.0
		+ 20.0)
		.min(420.0)
}
fn user_name(page: Option<&Page>, id: Id) -> String {
	page.and_then(|page| page.users.iter().find(|user| user.id == id))
		.map_or_else(|| id.to_string(), |user| user.name.clone())
}
fn summary(entry: &Entry, page: &Page, state: &State) -> String {
	let actor = entry
		.user_id
		.map(|id| user_name(Some(page), id))
		.unwrap_or_else(|| crate::i18n::translate("server-audit-log-summary-unknown-user"));
	let action = if action_text(entry.action_type) == "server-audit-log-event-icon-unknown-action" {
		format!(
			"{} {}",
			crate::i18n::translate("server-audit-log-summary-performed-action"),
			entry.action_type
		)
	} else {
		crate::i18n::translate_if_key(action_text(entry.action_type)).to_lowercase()
	};
	let name = entry
		.changes
		.iter()
		.find(|change| matches!(change.key.as_str(), "name" | "code"))
		.and_then(|change| match (&change.new, &change.old) {
			(Patch::Value(value), _) | (_, Patch::Value(value)) => Some(value.as_str()),
			_ => None,
		});
	let target = name
		.or_else(|| {
			entry
				.target_id
				.as_deref()
				.and_then(|target| target.parse::<Id>().ok())
				.and_then(|id| state.channel(id))
				.map(|channel| channel.name.as_str())
		})
		.or(entry.target_id.as_deref());
	if let Some(target) = target {
		format!("{actor} {action} {target}")
	} else {
		format!("{actor} {action}")
	}
}
fn timestamp(id: Id) -> String {
	let seconds = ((id.0 >> 22) + 1_420_070_400_000) / 1000;
	let Ok(utc) = time::OffsetDateTime::from_unix_timestamp(seconds as i64) else {
		return "Unknown time".into();
	};
	let local = crate::local_time::local(utc);
	let now = crate::local_time::now();
	let date = if local.date() == now.date() {
		"Today".into()
	} else if Some(local.date()) == now.date().previous_day() {
		"Yesterday".into()
	} else {
		local.date().to_string()
	};
	format!("{date} at {:02}:{:02}", local.hour(), local.minute())
}
fn details(ui: &mut egui::Ui, entry: &Entry, state: &State) {
	let colors = design::palette(ui);
	let mut index = 1;
	let mut line = |ui: &mut egui::Ui, text: String| {
		ui.horizontal_top(|ui| {
			ui.label(
				RichText::new(format!("{index:02}  -"))
					.monospace()
					.size(11.0)
					.color(colors.positive),
			);
			ui.add(egui::Label::new(text).wrap());
		});
		index += 1;
	};
	for change in &entry.changes {
		line(ui, change_text(change, state));
	}
	for option in &entry.options {
		line(ui, format!("{}: {}", field_name(&option.key), option.value));
	}
	if let Some(reason) = &entry.reason {
		line(ui, format!("Reason: {reason}"));
	}
	if index == 1 {
		ui.weak(crate::i18n::translate(
			"server-audit-log-details-no-additional-details-were-provided-for-this-event",
		));
	}
}
fn change_text(change: &Change, state: &State) -> String {
	if matches!(change.old, Patch::Absent)
		&& let Patch::Value(value) = &change.new
	{
		match change.key.as_str() {
			"code" => return format!("With code {value}"),
			"channel_id" => {
				if let Some(channel) = value.parse::<Id>().ok().and_then(|id| state.channel(id)) {
					return format!("For channel #{}", channel.name);
				}
			}
			"max_uses" if value == "0" => return "With unlimited uses".into(),
			"max_uses" => return format!("With a limit of {value} uses"),
			"max_age" if value == "0" => return "Which never expires".into(),
			"max_age" => {
				if let Ok(seconds) = value.parse::<u64>() {
					return if seconds % 86400 == 0 {
						format!("Which expires after {} days", seconds / 86400)
					} else if seconds % 3600 == 0 {
						format!("Which expires after {} hours", seconds / 3600)
					} else {
						format!("Which expires after {seconds} seconds")
					};
				}
			}
			"temporary" if value == "true" || value == "false" => {
				return format!(
					"With temporary membership {}",
					if value == "true" { "on" } else { "off" }
				);
			}
			_ => {}
		}
	}
	let field = field_name(&change.key);
	match (&change.old, &change.new) {
		(Patch::Absent, Patch::Value(value)) => format!("{field}: {value}"),
		(Patch::Value(value), Patch::Absent) => format!("Removed {field}: {value}"),
		(old, new) => format!("{field}: {} → {}", patch_text(old), patch_text(new)),
	}
}
fn patch_text(value: &Patch<String>) -> &str {
	match value {
		Patch::Absent => "Not provided",
		Patch::Null => "None",
		Patch::Value(value) => value,
	}
}
fn field_name(key: &str) -> String {
	key.replace('_', " ")
}
fn event_icon(action: u16) -> icons::Icon {
	match action {
		10..=15 => icons::Icon::Hash,
		20..=28 | 72..=74 => icons::Icon::People,
		30..=32 => icons::Icon::ShieldWarning,
		40..=42 => icons::Icon::Link,
		50..=52 | 80..=82 => icons::Icon::Activities,
		60..=62 => icons::Icon::Smile,
		110..=112 => icons::Icon::Thread,
		_ => icons::Icon::Gear,
	}
}
fn action_text(action: u16) -> &'static str {
	ACTIONS.iter().find(|(value, _)| *value == action).map_or(
		"server-audit-log-event-icon-unknown-action",
		|(_, label)| *label,
	)
}
const ACTIONS: &[(u16, &str)] = &[
	(1, "server-audit-log-action-text-updated-server-settings"),
	(10, "server-audit-log-action-text-created-channel"),
	(11, "server-audit-log-action-text-updated-channel"),
	(12, "server-audit-log-action-text-deleted-channel"),
	(
		13,
		"server-audit-log-action-text-created-channel-permission-overwrite",
	),
	(
		14,
		"server-audit-log-action-text-updated-channel-permission-overwrite",
	),
	(
		15,
		"server-audit-log-action-text-deleted-channel-permission-overwrite",
	),
	(20, "server-audit-log-action-text-kicked-member"),
	(21, "server-audit-log-action-text-pruned-members"),
	(22, "server-audit-log-action-text-banned-member"),
	(23, "server-audit-log-action-text-unbanned-member"),
	(24, "server-audit-log-action-text-updated-member"),
	(25, "server-audit-log-action-text-updated-member-roles"),
	(26, "server-audit-log-action-text-moved-member"),
	(27, "server-audit-log-action-text-disconnected-member"),
	(28, "server-audit-log-action-text-added-bot"),
	(30, "server-audit-log-action-text-created-role"),
	(31, "server-audit-log-action-text-updated-role"),
	(32, "server-audit-log-action-text-deleted-role"),
	(40, "server-audit-log-action-text-created-invite"),
	(41, "server-audit-log-action-text-updated-invite"),
	(42, "server-audit-log-action-text-deleted-invite"),
	(50, "server-audit-log-action-text-created-webhook"),
	(51, "server-audit-log-action-text-updated-webhook"),
	(52, "server-audit-log-action-text-deleted-webhook"),
	(60, "server-audit-log-action-text-created-emoji"),
	(61, "server-audit-log-action-text-updated-emoji"),
	(62, "server-audit-log-action-text-deleted-emoji"),
	(72, "server-audit-log-action-text-deleted-message"),
	(73, "server-audit-log-action-text-deleted-messages"),
	(74, "server-audit-log-action-text-pinned-message"),
	(75, "server-audit-log-action-text-unpinned-message"),
	(80, "server-audit-log-action-text-created-integration"),
	(81, "server-audit-log-action-text-updated-integration"),
	(82, "server-audit-log-action-text-deleted-integration"),
	(83, "server-audit-log-action-text-created-stage"),
	(84, "server-audit-log-action-text-updated-stage"),
	(85, "server-audit-log-action-text-deleted-stage"),
	(90, "server-audit-log-action-text-created-sticker"),
	(91, "server-audit-log-action-text-updated-sticker"),
	(92, "server-audit-log-action-text-deleted-sticker"),
	(100, "server-audit-log-action-text-created-scheduled-event"),
	(101, "server-audit-log-action-text-updated-scheduled-event"),
	(102, "server-audit-log-action-text-deleted-scheduled-event"),
	(110, "server-audit-log-action-text-created-thread"),
	(111, "server-audit-log-action-text-updated-thread"),
	(112, "server-audit-log-action-text-deleted-thread"),
	(
		121,
		"server-audit-log-action-text-updated-application-command-permissions",
	),
	(130, "server-audit-log-action-text-created-soundboard-sound"),
	(131, "server-audit-log-action-text-updated-soundboard-sound"),
	(132, "server-audit-log-action-text-deleted-soundboard-sound"),
	(140, "server-audit-log-action-text-created-automod-rule"),
	(141, "server-audit-log-action-text-updated-automod-rule"),
	(142, "server-audit-log-action-text-deleted-automod-rule"),
	(
		143,
		"server-audit-log-action-text-blocked-message-with-automod",
	),
	(
		144,
		"server-audit-log-action-text-flagged-message-with-automod",
	),
	(
		145,
		"server-audit-log-action-text-timed-out-member-with-automod",
	),
	(
		146,
		"server-audit-log-action-text-quarantined-member-with-automod",
	),
	(
		150,
		"server-audit-log-action-text-created-creator-monetization-request",
	),
	(
		151,
		"server-audit-log-action-text-accepted-creator-monetization-terms",
	),
	(
		163,
		"server-audit-log-action-text-created-onboarding-prompt",
	),
	(
		164,
		"server-audit-log-action-text-updated-onboarding-prompt",
	),
	(
		165,
		"server-audit-log-action-text-deleted-onboarding-prompt",
	),
	(166, "server-audit-log-action-text-created-onboarding"),
	(167, "server-audit-log-action-text-updated-onboarding"),
	(190, "server-audit-log-action-text-created-home-settings"),
	(191, "server-audit-log-action-text-updated-home-settings"),
	(
		192,
		"server-audit-log-action-text-created-voice-channel-status",
	),
	(
		193,
		"server-audit-log-action-text-deleted-voice-channel-status",
	),
];
#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn audit_layout_fits_and_entry_expansion_is_bounded() {
		for width in [280.0, 360.0, 800.0] {
			let ctx = egui::Context::default();
			design::apply(&ctx);
			let mut state = test_support::demo_state();
			let guild = state.guilds[0].id;
			state.server_admin.guild = Some(guild);
			state.server_admin.audit_log = Some(Page {
				guild,
				users: vec![],
				has_more: false,
				entries: vec![Entry {
					id: Id(123),
					user_id: None,
					target_id: None,
					action_type: 40,
					reason: Some("A long synthetic audit reason that wraps. ".repeat(20)),
					options: vec![],
					changes: vec![Change {
						key: "code".into(),
						old: Patch::Absent,
						new: Patch::Value("synthetic".into()),
					}],
				}],
			});
			let mut view = AuditLogUi {
				expanded: Some(Id(123)),
				..Default::default()
			};
			let output = ctx.run_ui(
				egui::RawInput {
					screen_rect: Some(egui::Rect::from_min_size(
						egui::Pos2::ZERO,
						Vec2::new(width, 900.0),
					)),
					..Default::default()
				},
				|ui| {
					ui.set_width(width);
					ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
					let right = ui.max_rect().right();
					let mut commands = vec![];
					view.show(
						ui,
						&mut state,
						guild,
						&mut Avatars::default(),
						&mut commands,
					);
					assert!(commands.is_empty());
					assert!(
						ui.min_rect().right() <= right + 1.0,
						"audit overflow at {width}: {:?}",
						ui.min_rect()
					);
				},
			);
			output.drop_without_applying_deltas();
			assert!(
				details_height(&state.server_admin.audit_log.as_ref().unwrap().entries[0]) <= 420.0
			);
		}
	}

	#[test]
	fn audit_details_preserve_absent_null_and_invite_values() {
		let state = State::default();
		let mut change = Change {
			key: "max_age".into(),
			old: Patch::Absent,
			new: Patch::Value("2592000".into()),
		};
		assert_eq!(change_text(&change, &state), "Which expires after 30 days");
		change.new = Patch::Null;
		assert_eq!(change_text(&change, &state), "max age: Not provided → None");
		change.old = Patch::Value("3600".into());
		change.new = Patch::Absent;
		assert_eq!(change_text(&change, &state), "Removed max age: 3600");
		assert_eq!(
			crate::i18n::Language::English.text(action_text(65535)),
			"Unknown Action"
		);
	}
}
