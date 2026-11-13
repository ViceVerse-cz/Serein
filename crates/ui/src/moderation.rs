//! Timeout, kick and ban confirmations opened from a server member's context menu.
use client_core::{Command, State};
use model::{Id, User, server_admin::Action};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
	Timeout,
	Kick,
	Ban,
}

/// Timeout lengths in seconds, the steps Discord offers; zero removes the timeout.
const TIMEOUTS: [(u32, &str); 7] = [
	(60, "moderation-duration-60-seconds"),
	(300, "forum-settings-module-5-minutes"),
	(600, "forum-settings-module-10-minutes"),
	(3600, "polls-duration-1-hour"),
	(86_400, "server-invite-expiry-label-1-day"),
	(604_800, "polls-duration-1-week"),
	(0, "moderation-remove-timeout"),
];
/// Seconds of the member's recent messages that a ban deletes.
const HISTORY: [(u32, &str); 4] = [
	(0, "moderation-delete-none"),
	(3600, "moderation-delete-hour"),
	(86_400, "moderation-delete-day"),
	(604_800, "moderation-delete-week"),
];

impl Kind {
	fn options(self) -> &'static [(u32, &'static str)] {
		match self {
			Self::Timeout => &TIMEOUTS,
			Self::Kick => &[],
			Self::Ban => &HISTORY,
		}
	}
	/// The request for `user` with the option at `choice` selected.
	pub(crate) fn action(self, user: Id, choice: usize) -> Action {
		let seconds = self.options().get(choice).map_or(0, |option| option.0);
		match self {
			Self::Timeout => Action::Timeout {
				user,
				seconds: (seconds != 0).then_some(seconds),
			},
			Self::Kick => Action::Kick { user },
			Self::Ban => Action::Ban {
				user,
				delete_seconds: seconds,
			},
		}
	}
	/// The menu row, which is also the dialog title; takes the member's `$name`.
	pub(crate) fn menu_key(self) -> &'static str {
		match self {
			Self::Timeout => "moderation-menu-timeout",
			Self::Kick => "moderation-menu-kick",
			Self::Ban => "moderation-menu-ban",
		}
	}
}

struct Target {
	guild: Id,
	user: User,
	kind: Kind,
	choice: usize,
	generation: u64,
}

#[derive(Default)]
pub(super) struct Moderation {
	target: Option<Target>,
	/// A request is in flight; its result closes the dialog or becomes `error`.
	submitted: bool,
	error: Option<&'static str>,
}

impl Moderation {
	pub fn open(&mut self, guild: Id, user: User, kind: Kind, state: &State) {
		if self.submitted {
			return;
		}
		self.error = None;
		self.target = Some(Target {
			guild,
			user,
			kind,
			choice: 0,
			generation: state.generation,
		});
	}
	/// `settings_open` leaves the shared administration state to Server Settings.
	pub fn show(
		&mut self,
		ctx: &egui::Context,
		state: &mut State,
		commands: &mut Vec<Command>,
		settings_open: bool,
	) {
		if self.submitted && !state.server_admin.pending {
			self.submitted = false;
			self.error = state.server_admin.error;
			// The failure is kept here, so drop the reload fence it would leave behind.
			if !settings_open {
				state.close_server_admin();
			}
			if self.error.is_none() {
				self.target = None;
			}
		}
		let Some(target) = &mut self.target else {
			return;
		};
		if target.generation != state.generation {
			*self = Self::default();
			return;
		}
		let (guild, kind) = (target.guild, target.kind);
		let action = kind.action(target.user.id, target.choice);
		let ready = !self.submitted
			&& !state.server_admin.pending
			&& !state.server_admin.needs_refresh
			&& !state.server_settings.saving
			&& (state.demo
				|| (state.gateway_connected
					&& state.auth == client_core::auth::AuthState::Authenticated))
			&& state.server_admin_action_allowed(guild, &action);
		let confirm = match &action {
			Action::Timeout { seconds: None, .. } => "server-admin-dialog-save",
			Action::Timeout { .. } => "moderation-confirm-timeout",
			Action::Ban { .. } => "moderation-confirm-ban",
			_ => "server-admin-member-menu-kick",
		};
		let (detail, label) = match kind {
			Kind::Timeout => ("moderation-timeout-detail", "polls-creator-duration"),
			Kind::Kick => ("moderation-kick-detail", ""),
			Kind::Ban => ("moderation-ban-detail", "moderation-delete-history"),
		};
		let title = crate::i18n::translate_args(kind.menu_key(), &[("name", &target.user.name)]);
		let error = self.error;
		let choice = &mut target.choice;
		let (mut confirmed, mut cancelled) = (false, false);
		let response = crate::dialog::Dialog::new("member-moderation", title)
			.subtitle(crate::i18n::translate(detail))
			.width(440.0)
			.danger()
			.show(ctx, |d| {
				d.content(|ui| {
					ui.spacing_mut().item_spacing.y = 10.0;
					let options = kind.options();
					if let Some(selected) = options.get(*choice) {
						crate::dialog::label(ui, label);
						crate::design::Select::from_id_salt("member-moderation-option")
							.selected_text(crate::i18n::translate(selected.1))
							.width(ui.available_width())
							.show_ui(ui, |ui| {
								for (index, option) in options.iter().enumerate() {
									ui.selectable_value(
										choice,
										index,
										crate::i18n::translate(option.1),
									);
								}
							});
					}
					if let Some(error) = error {
						crate::dialog::notice(ui, crate::dialog::Level::Error, error);
					}
				});
				d.footer(|ui| {
					ui.add_enabled_ui(ready, |ui| {
						confirmed =
							crate::dialog::action(ui, confirm, crate::dialog::Action::Danger)
								.clicked();
					});
					cancelled = crate::dialog::action(
						ui,
						"server-admin-dialog-cancel",
						crate::dialog::Action::Neutral,
					)
					.clicked();
				});
			});
		if confirmed && let Some(command) = state.request_server_admin(guild, action) {
			commands.push(command);
			self.submitted = true;
			self.error = None;
		} else if (cancelled || response.close) && !self.submitted {
			*self = Self::default();
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use client_core::auth::Failure;
	use egui::{Event, Modifiers, PointerButton, Pos2, Rect};

	fn labels(shape: &egui::Shape, out: &mut Vec<(String, Rect)>) {
		match shape {
			egui::Shape::Text(t) => out.push((
				t.galley.job.text.clone(),
				t.galley.rect.translate(t.pos.to_vec2()),
			)),
			egui::Shape::Vec(shapes) => shapes.iter().for_each(|s| labels(s, out)),
			_ => {}
		}
	}
	struct Harness {
		ctx: egui::Context,
		state: State,
		view: Moderation,
		commands: Vec<Command>,
	}
	impl Harness {
		fn frame(&mut self, events: Vec<Event>) -> Vec<(String, Rect)> {
			let mut output = self.ctx.run_ui(
				egui::RawInput {
					screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(900.0, 700.0))),
					events,
					..Default::default()
				},
				|ui| {
					self.view
						.show(ui.ctx(), &mut self.state, &mut self.commands, false)
				},
			);
			output.textures_delta.clear();
			let mut text = vec![];
			for shape in output.shapes {
				labels(&shape.shape, &mut text);
			}
			text
		}
		/// Clicks the confirming button and returns the request it sent.
		fn confirm(&mut self, label: &str) -> (Id, u64, Action) {
			let text = self.frame(vec![]);
			let (_, rect) = text
				.iter()
				.find(|(text, _)| text == label)
				.unwrap_or_else(|| panic!("no {label} button in {text:?}"));
			for pressed in [true, false] {
				self.frame(vec![
					Event::PointerMoved(rect.center()),
					Event::PointerButton {
						pos: rect.center(),
						button: PointerButton::Primary,
						pressed,
						modifiers: Modifiers::NONE,
					},
				]);
			}
			let Some(Command::ServerAdmin {
				guild,
				request,
				action,
			}) = self.commands.pop()
			else {
				panic!("{label} sends a request")
			};
			(guild, request, *action)
		}
		fn deliver(&mut self, request: u64, result: Result<model::server_admin::Result, Failure>) {
			self.state.apply(client_core::Envelope {
				generation: self.state.generation,
				event: client_core::Event::ServerAdmin(client_core::server_admin::Event {
					guild: Id(10),
					request,
					result,
				}),
			});
		}
	}

	#[test]
	fn a_refused_ban_stays_open_with_its_failure_and_a_retry_that_succeeds_closes() {
		// You own the synthetic server; Mika is listed in the open channel.
		let mut state = test_support::demo_state();
		let own = state.user.clone().unwrap();
		let mut permissions = test_support::permission_snapshot(&state);
		for guild in &mut permissions.guilds {
			guild.owner = Some(own.id);
		}
		state.apply(client_core::Envelope {
			generation: state.generation,
			event: client_core::Event::Permissions(client_core::permissions::Event::Snapshot(
				permissions,
			)),
		});
		let mika = User {
			id: Id(77),
			name: "Mika".into(),
			..own
		};
		state.members = Some(model::MemberList {
			guild: Some(Id(10)),
			channel: state.selected.unwrap(),
			request: 0,
			start: 0,
			slots: vec![Some(model::MemberSlot::Person(model::Member {
				roles: vec![],
				user: mika.clone(),
				nick: None,
				status: None,
				custom_status: None,
				activities: vec![],
				clients: Default::default(),
			}))],
			total: 1,
			lazy: true,
			freshness: model::Freshness::Fresh,
			groups: vec![],
			ranges: vec![],
		});
		let mut h = Harness {
			ctx: egui::Context::default(),
			state,
			view: Moderation::default(),
			commands: vec![],
		};
		h.view.open(Id(10), mika, Kind::Ban, &h.state);

		let (guild, request, action) = h.confirm("Ban");
		assert_eq!(guild, Id(10));
		assert!(matches!(
			action,
			Action::Ban {
				user: Id(77),
				delete_seconds: 0
			}
		));
		h.deliver(request, Err(Failure::Forbidden));
		let text = h.frame(vec![]);
		assert!(
			text.iter()
				.any(|(text, _)| text.contains(Failure::Forbidden.label())),
			"the refusal is shown in the dialog: {text:?}"
		);

		let (_, request, _) = h.confirm("Ban");
		h.deliver(request, Ok(model::server_admin::Result::Kicked(Id(77))));
		assert!(h.frame(vec![]).is_empty(), "a completed ban closes");
		assert!(!h.state.server_admin.pending && h.state.server_admin.guild.is_none());
	}
}
