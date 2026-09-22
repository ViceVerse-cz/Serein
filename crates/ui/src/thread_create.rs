//! One bounded "start a thread" request at a time, from a message or from the channel header.
use crate::dialog;
use client_core::{Command, State, channel_actions::Action};
use model::Id;

struct Request {
	channel: Id,
	/// The message the thread hangs off, when it was started from one.
	message: Option<Id>,
	name: String,
	submitted: bool,
}

/// First line of the starter message, collapsed whitespace, cut at a word boundary near 60 chars.
pub(crate) fn suggested_name(text: &str) -> String {
	let line = text
		.lines()
		.map(str::trim)
		.find(|line| !line.is_empty())
		.unwrap_or_default();
	let words: Vec<&str> = line.split_whitespace().collect();
	let mut out = String::new();
	for word in words {
		let next = out.chars().count() + word.chars().count() + usize::from(!out.is_empty());
		if next > 60 {
			break;
		}
		if !out.is_empty() {
			out.push(' ');
		}
		out.push_str(word);
	}
	if out.is_empty() {
		line.chars().take(60).collect()
	} else {
		out
	}
}

#[derive(Default)]
pub(crate) struct ThreadCreateUi {
	request: Option<Request>,
	generation: u64,
}

impl ThreadCreateUi {
	/// Opens the editor for `channel`, optionally starting from one of its messages.
	pub fn open(&mut self, channel: Id, message: Option<Id>, name: String) {
		self.request = Some(Request {
			channel,
			message,
			name: name.chars().take(100).collect(),
			submitted: false,
		});
	}
	pub fn show(&mut self, ctx: &egui::Context, state: &mut State, commands: &mut Vec<Command>) {
		if self.generation != state.generation {
			self.generation = state.generation;
			self.request = None;
			return;
		}
		let Some(request) = &mut self.request else {
			return;
		};
		let channel = request.channel;
		if !state.can_create_thread(channel) && !request.submitted {
			self.request = None;
			return;
		}
		if request.submitted && state.channel_action_succeeded(channel) {
			// The reducer admitted the new thread; open it like Discord does.
			let name = request.name.trim().to_owned();
			let created = state
				.channels
				.iter()
				.filter(|c| {
					c.parent_id == Some(channel)
						&& matches!(c.kind, 10..=12)
						&& c.name.trim() == name
				})
				.map(|c| c.id)
				.max();
			self.request = None;
			if let Some(id) = created
				&& let Some(command) = state.select(id)
			{
				commands.push(command);
			}
			return;
		}
		let mut close = false;
		let result = dialog::Dialog::new(("create-thread", channel), "Create Thread")
			.subtitle(if request.message.is_some() {
				"The selected message starts the thread. Everyone who can see this channel can see the thread."
			} else {
				"Everyone who can see this channel can see the thread."
			})
			.width(420.0)
			.show(ctx, |d| {
				d.content(|ui| {
					let label = dialog::label(ui, "Thread name");
					dialog::input(
						ui,
						egui::TextEdit::singleline(&mut request.name).char_limit(100),
					)
					.labelled_by(label.id);
					request.name.shrink_to_fit();
					if let Some(error) = state
						.channel_action_status(channel)
						.filter(|_| !state.channel_action_succeeded(channel))
					{
						dialog::notice(ui, dialog::Level::Error, error);
					}
				});
				d.footer(|ui| {
					ui.add_enabled_ui(
						!state.channel_action_pending()
							&& (state.demo || state.gateway_connected)
							&& client_core::channel_actions::valid_name(&request.name),
						|ui| {
							if dialog::action(ui, "Create", dialog::Action::Primary).clicked() {
								let action = Action::CreateThread {
									name: request.name.trim().to_owned(),
									message: request.message,
								};
								if let Some(command) = state.request_channel_action(channel, action)
								{
									commands.push(command);
									request.submitted = true;
								}
							}
						},
					);
					close = dialog::action(ui, "Cancel", dialog::Action::Neutral).clicked();
				});
			});
		if close || result.close {
			self.request = None;
		}
	}
}
