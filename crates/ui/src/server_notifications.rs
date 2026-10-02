//! One session-only draft; saving changes only the controls the user touched.
use crate::{design, dialog, i18n::translate, icons};
use client_core::{Command, State, server_actions::NotificationOptions};
use model::Id;

#[derive(Default)]
pub(super) struct Editor {
	edits: NotificationOptions,
	submitted: bool,
}
impl Editor {
	pub fn show(
		&mut self,
		ctx: &egui::Context,
		state: &mut State,
		guild: Id,
		name: &str,
		commands: &mut Vec<Command>,
	) -> bool {
		let current = state.server_notification_settings(guild);
		let pending = state.server_action_pending();
		if self.submitted && !pending {
			// Show the reconciled account state after completion, including a newer
			// Gateway update. Errors remain visible; another write needs a new choice.
			self.submitted = false;
			self.edits = NotificationOptions::default();
		}
		let available = state.can_update_server_notifications(guild);
		let mut save = false;
		let mut close = false;
		let response = dialog::Dialog::new(
			"server-notification-settings",
			translate("server-notifications-title"),
		)
		.subtitle(name)
		.icon(icons::Icon::Bell)
		.width(440.0)
		.show(ctx, |d| {
			let max_height = (d.available_height() - 220.0).max(100.0);
			d.content(|ui| {
				egui::ScrollArea::vertical()
					.max_height(max_height)
					.show(ui, |ui| {
						ui.add_enabled_ui(available && !pending, |ui| {
							let level = self.edits.level.or(current.level);
							let default = match state
								.guild(guild)
								.and_then(|g| g.default_message_notifications)
							{
								Some(0) => "server-notifications-default-all",
								Some(1) => "server-notifications-default-mentions",
								_ => "server-notifications-default-unknown",
							};
							for (value, label) in [
								(3, "server-notifications-default"),
								(0, "server-notifications-all"),
								(1, "server-notifications-mentions"),
								(2, "server-notifications-nothing"),
							] {
								if design::radio_row(
									ui,
									level == Some(value),
									label,
									(value == 3).then_some(default),
								)
								.clicked()
								{
									self.edits.level = Some(value);
								}
							}
							design::card_divider(ui);
							for (label, original, edit) in [
								(
									"server-notifications-mute",
									current.muted,
									&mut self.edits.muted,
								),
								(
									"server-notifications-everyone",
									current.suppress_everyone,
									&mut self.edits.suppress_everyone,
								),
								(
									"server-notifications-roles",
									current.suppress_roles,
									&mut self.edits.suppress_roles,
								),
							] {
								let mut value = edit.or(original).unwrap_or(false);
								if design::switch(ui, label, None, &mut value).changed() {
									*edit = Some(value);
								}
							}
						});
						dialog::hint(ui, "server-notifications-overrides");
						if current.level.is_none() || current.muted.is_none() {
							dialog::notice(
								ui,
								dialog::Level::Warning,
								"server-notifications-unknown",
							);
						}
						if !available && !pending {
							dialog::notice(
								ui,
								dialog::Level::Warning,
								"server-notifications-offline",
							);
						}
						if let Some(status) = state.server_action_status(guild) {
							dialog::hint(ui, status);
						}
						if state.demo {
							dialog::hint(ui, "server-menu-show-offline-preview-no-server-changes");
						}
					});
			});
			d.footer(|ui| {
				ui.add_enabled_ui(
					available && !pending && self.edits != NotificationOptions::default(),
					|ui| {
						save = dialog::action(
							ui,
							if pending {
								"server-notifications-saving"
							} else {
								"server-notifications-save"
							},
							dialog::Action::Primary,
						)
						.clicked();
					},
				);
				close =
					dialog::action(ui, "server-menu-show-close", dialog::Action::Neutral).clicked();
			});
		});
		if save && let Some(command) = state.update_server_notifications(guild, self.edits) {
			commands.push(command);
			self.submitted = true;
		}
		close || response.close
	}
}
