//! Account messaging preferences in the existing settings shell.
use crate::{MessagingUi, design};
use client_core::{Command, State};
use egui::RichText;
use model::{
	Id,
	messaging_permissions::{Change, MAX_GUILDS},
};

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(super) enum Tab {
	#[default]
	Spam,
	DirectMessages,
	FriendRequests,
	ConnectedGames,
}
impl Tab {
	pub const ALL: [Self; 4] = [
		Self::Spam,
		Self::DirectMessages,
		Self::FriendRequests,
		Self::ConnectedGames,
	];
	pub fn label(self) -> &'static str {
		match self {
			Self::Spam => "Spam Filters",
			Self::DirectMessages => "Direct Messages",
			Self::FriendRequests => "Friend Requests",
			Self::ConnectedGames => "Connected Games",
		}
	}
	fn heading(self) -> &'static str {
		match self {
			Self::Spam => "Spam Filters",
			Self::DirectMessages => "Direct Message (DM) Permissions",
			Self::FriendRequests => "Friend Request Permissions",
			Self::ConnectedGames => "Messaging in Connected Games",
		}
	}
}
#[derive(Default)]
pub(super) struct Navigation {
	pub active: Tab,
	pub jump: Option<Tab>,
	pub requested: bool,
	generation: u64,
	guild: Option<Id>,
}
impl Navigation {
	fn heading(&mut self, ui: &mut egui::Ui, tab: Tab) {
		if tab != Tab::Spam {
			ui.add_space(12.0);
		}
		let heading = ui.label(design::eyebrow(
			ui,
			tab.heading(),
			design::palette(ui).muted,
		));
		if heading.rect.top() <= ui.clip_rect().top() + 28.0 {
			self.active = tab;
		}
		if self.jump == Some(tab) {
			ui.scroll_to_rect(heading.rect.expand(8.0), Some(egui::Align::Min));
			self.jump = None;
		}
	}
}
fn toggle(ui: &mut egui::Ui, label: &str, detail: Option<&str>, mut value: bool) -> Option<bool> {
	design::switch(ui, label, detail, &mut value)
		.changed()
		.then_some(value)
}
fn detail(ui: &mut egui::Ui, text: &str) {
	ui.label(
		RichText::new(text)
			.size(13.0)
			.color(design::palette(ui).muted),
	);
}
impl MessagingUi {
	pub(super) fn messaging_permissions_settings(
		&mut self,
		ui: &mut egui::Ui,
		state: &mut State,
		commands: &mut Vec<Command>,
	) {
		let nav = &mut self.settings.messaging_permissions;
		if nav.generation != state.generation {
			*nav = Navigation {
				generation: state.generation,
				..Default::default()
			};
		}
		if state.messaging_permissions.snapshot.is_none()
			&& !state.messaging_permissions.pending
			&& state.messaging_permissions.error.is_none()
		{
			nav.requested = false;
		}
		if !nav.requested && !state.messaging_permissions.pending {
			if let Some(command) = state.request_messaging_permissions() {
				commands.push(command);
			}
			nav.requested = true;
		}
		if nav
			.guild
			.is_some_and(|id| !state.guilds.iter().any(|guild| guild.id == id))
		{
			nav.guild = None;
		}
		if ui.available_width() < 500.0 {
			ui.horizontal_wrapped(|ui| {
				for tab in Tab::ALL {
					if ui
						.selectable_label(nav.active == tab, tab.label())
						.clicked()
					{
						nav.jump = Some(tab);
					}
				}
			});
		}
		let busy = state.messaging_permissions.pending;
		if busy {
			ui.horizontal(|ui| {
				ui.add(egui::Spinner::new().size(14.0));
				detail(
					ui,
					if state.messaging_permissions.snapshot.is_some() {
						"Saving…"
					} else {
						"Loading your preferences…"
					},
				);
			});
		}
		if let Some(error) = state.messaging_permissions.error {
			design::notice(ui, design::Level::Error, error.label());
			if !busy && design::text_action(ui, "Try again").clicked() {
				nav.requested = false;
			}
		}
		let Some(settings) = state.messaging_permissions.snapshot.as_ref() else {
			return;
		};
		let mut change = None;
		ui.add_enabled_ui(!busy, |ui| {
			nav.heading(ui, Tab::Spam);
			design::card(ui, |ui| {
				design::section(
					ui,
					"Automatically filter suspected spam messages",
					Some("Discord can filter out some messages that contain spam. These messages go to your Spam inbox."),
				);
				for (value, label, hint) in [
					(3, "Filter all spam", None),
					(2, "Filter messages from non-friends", Some("Recommended")),
					(1, "Don't filter spam", None),
				] {
					let selected = settings.spam_filter == value || (settings.spam_filter == 0 && value == 2);
					if design::radio_row(ui, selected, label, hint).clicked() && !selected {
						change = Some(Change::SpamFilter(value));
					}
				}
				if settings.spam_filter > 3 {
					design::hint(ui, "Your account uses a custom spam filter setting. Select an option to replace it.");
				}
			});

			nav.heading(ui, Tab::DirectMessages);
			design::card(ui, |ui| {
				let label = nav.guild.and_then(|id| state.guilds.iter().find(|guild| guild.id == id)).map_or("All servers", |guild| guild.name.as_str()).to_owned();
				let allow = settings.allow_dms(nav.guild);
				let filter = settings.filter_requests(nav.guild);
				let all = nav.guild.is_none();
				let mixed = all && state.guilds.iter().any(|guild| settings.allow_dms(Some(guild.id)) != allow || settings.filter_requests(Some(guild.id)) != filter);
				design::row(
					ui,
					"Server",
					Some(if mixed {
						"Some servers have different preferences. Choose a server to review its settings."
					} else if all {
						"Changes apply to all current servers and set the default for newly joined servers."
					} else {
						"Changes apply to this server only."
					}),
					|ui| {
						egui::ComboBox::from_id_salt("messaging-permissions-guild")
							.selected_text(label)
							.width(ui.available_width().min(260.0))
							.height(280.0)
							.show_ui(ui, |ui| {
								ui.selectable_value(&mut nav.guild, None, "All servers");
								for guild in &state.guilds {
									ui.selectable_value(&mut nav.guild, Some(guild.id), &guild.name);
								}
							});
					},
				);
				design::card_divider(ui);
				ui.add_enabled_ui(!all || state.guilds.len() <= MAX_GUILDS, |ui| {
					if let Some(enabled) = toggle(ui, "Allow DMs from other server members", None, allow) {
						change = Some(match nav.guild {
							Some(id) => Change::AllowGuildDms(id, enabled),
							None => Change::AllowAllDms { guilds: state.guilds.iter().map(|guild| guild.id).collect(), enabled },
						});
					}
					design::card_divider(ui);
					if let Some(enabled) = toggle(ui, "Filter messages from server members I may not know", Some("Move messages from people you may not know into Message Requests."), filter) {
						change = Some(match nav.guild {
							Some(id) => Change::FilterGuildRequests(id, enabled),
							None => Change::FilterAllRequests { guilds: state.guilds.iter().map(|guild| guild.id).collect(), enabled },
						});
					}
				});
				if all && state.guilds.len() > MAX_GUILDS {
					design::hint(ui, "There are too many servers to update together. Choose an individual server.");
				}
			});

			nav.heading(ui, Tab::FriendRequests);
			design::card(ui, |ui| {
				design::section(ui, "Allow friend requests from", Some("Control who can send you friend requests and how they appear."));
				for (label, bit, make, description) in [
					("Everyone", 8, Change::Everyone as fn(bool) -> Change, None),
					("Friends of friends", 2, Change::FriendsOfFriends, None),
					("Server members", 4, Change::ServerMembers, Some("Only from servers where you also allow Direct Messages.")),
				] {
					if let Some(value) = toggle(ui, label, description, settings.friend_source_flags & bit != 0) {
						change = Some(make(value));
					}
				}
				design::card_divider(ui);
				if let Some(value) = toggle(ui, "Show personalized messages", Some("Show personalized messages on incoming friend requests. If you accept, the message will still appear in your DMs."), settings.personalized_requests) {
					change = Some(Change::PersonalizedRequests(value));
				}
			});

			nav.heading(ui, Tab::ConnectedGames);
			design::card(ui, |ui| {
				design::hint(ui, "Settings for games that use Discord to power their social experiences.");
				ui.add_space(4.0);
				if let Some(value) = toggle(ui, "Allow friends from games to send direct messages and invites", Some("Let friends from connected games send DMs and invite you to play, even when the game isn't open."), settings.game_friend_dms) {
					change = Some(Change::GameFriendDms(value));
				}
				design::card_divider(ui);
				design::section(ui, "Show Direct Messages in games", Some("Read and respond to DMs directly from in-game chats."));
				for (value, label) in [(1, "Show all DMs"), (2, "Show only DMs from people who also play the game"), (3, "Don't show DMs")] {
					let selected = settings.game_dms == value || (settings.game_dms == 0 && value == 1);
					if design::radio_row(ui, selected, label, None).clicked() && !selected {
						change = Some(Change::GameDms(value));
					}
				}
				if settings.game_dms > 3 {
					design::hint(ui, "Your account uses a custom in-game DM setting. Select an option to replace it.");
				}
			});
		});
		if let Some(change) = change
			&& let Some(command) = state.update_messaging_permissions(change)
		{
			commands.push(command);
		}
	}
}
