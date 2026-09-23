//! Slash commands: the `/` picker, option chips above the composer and private ("Only you can
//! see this") replies. Discovery, validation and submission all go through the reducer
//! (`request_application_commands`, `prepare_application_command`); nothing here builds a
//! protocol request. The offline demo answers from the synthetic catalog below.
use crate::input::{self, Input};
use crate::sidebar::avatar;
use crate::theme::{FONT, Icon, color, icon, palette, tint};
use crate::{Serein, autocomplete, channel_label};
use client_core::{Command, Envelope, Event, State, auth::AuthState, interactions};
use gpui::{prelude::*, *};
use model::{
	Id,
	application_commands::{self as schema, CommandOption, Value},
};
use std::time::Instant;

/// Picker matches kept per query, as in the main app.
const RESULTS: usize = 64;
/// Picker rows shown at once; the window follows the keyboard selection.
const ROWS: usize = 8;
/// Entries offered by a user, channel or role chooser.
const CANDIDATES: usize = 25;
/// Private replies shown above the composer.
const REPLIES: usize = 3;
#[cfg(target_os = "macos")]
const MONO: &str = "Menlo";
#[cfg(target_os = "windows")]
const MONO: &str = "Consolas";
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
const MONO: &str = "DejaVu Sans Mono";

/// One runnable command or subcommand from the index.
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
	pub id: Id,
	pub path: Vec<String>,
	/// `name`, `name sub` or `name group sub`, without the slash.
	pub name: String,
	pub description: String,
	pub application: String,
	pub application_id: Id,
}

/// The search typed after a leading `/`; `None` for any other draft.
pub fn query(draft: &str) -> Option<String> {
	let query = draft.strip_prefix('/')?;
	(query.len() <= 128 && !query.contains('\n')).then(|| query.trim_end().to_lowercase())
}

/// Usable commands and subcommands matching `query` by name, description or application,
/// grouped by application and then sorted by name.
pub fn entries(state: &State, channel: Id, query: &str) -> Vec<Entry> {
	let mut entries = Vec::new();
	for command in &state.application_commands.commands {
		if entries.len() >= RESULTS {
			break;
		}
		if !state.can_use_application_command(channel, command) {
			continue;
		}
		let mut leaves = Vec::new();
		if command.options.iter().any(|option| option.kind <= 2) {
			for option in &command.options {
				if option.kind == 1 {
					leaves.push((vec![option.name.clone()], option.description.as_str()));
				} else if option.kind == 2 {
					for child in &option.options {
						leaves.push((
							vec![option.name.clone(), child.name.clone()],
							child.description.as_str(),
						));
					}
				}
			}
		} else {
			leaves.push((Vec::new(), command.description.as_str()));
		}
		for (path, description) in leaves {
			let name = if path.is_empty() {
				command.name.clone()
			} else {
				format!("{} {}", command.name, path.join(" "))
			};
			if [name.as_str(), description, &command.application_name]
				.iter()
				.any(|text| text.to_lowercase().contains(query))
			{
				entries.push(Entry {
					id: command.id,
					path,
					name,
					description: description.into(),
					application: command.application_name.clone(),
					application_id: command.application_id,
				});
				if entries.len() >= RESULTS {
					break;
				}
			}
		}
	}
	entries.sort_by(|a, b| {
		(&a.application, a.application_id, &a.name).cmp(&(
			&b.application,
			b.application_id,
			&b.name,
		))
	});
	entries
}

/// The offline demo has no gateway session, which the reducer's application-command paths
/// require; there they run as a connected session and the flags are restored afterwards.
pub(crate) fn session<R>(state: &mut State, run: impl FnOnce(&mut State) -> R) -> R {
	if !state.demo {
		return run(state);
	}
	let saved = (state.auth, state.gateway_connected, state.freshness);
	state.auth = AuthState::Authenticated;
	state.gateway_connected = true;
	state.freshness = model::Freshness::Fresh;
	let result = run(state);
	(state.auth, state.gateway_connected, state.freshness) = saved;
	result
}

fn value_text(value: &Value) -> String {
	match value {
		Value::String(value) => value.clone(),
		Value::Integer(value) => value.to_string(),
		Value::Number(value) => value.to_string(),
		Value::Boolean(value) => value.to_string(),
	}
}

/// Chooser entries (label, value) for a boolean, choice, user, channel, role or mentionable.
fn candidates(state: &State, channel: Id, option: &CommandOption) -> Vec<(String, String)> {
	let mut out = Vec::new();
	if option.kind == 5 {
		out.push(("True".into(), "true".into()));
		out.push(("False".into(), "false".into()));
	}
	for choice in &option.choices {
		out.push((choice.name.clone(), value_text(&choice.value)));
	}
	if matches!(option.kind, 6 | 9) {
		let people = autocomplete::items_up_to(state, autocomplete::Kind::Person, "", CANDIDATES);
		out.extend(people.into_iter().filter_map(|item| {
			let user = item.user?;
			Some((format!("@{}", item.label), user.id.to_string()))
		}));
	}
	let guild = state.channel(channel).and_then(|c| c.guild);
	if option.kind == 7 {
		out.extend(
			state
				.channels
				.iter()
				.filter(|target| {
					guild.is_some()
						&& target.guild == guild
						&& state.can_view(target.id)
						&& (option.channel_types.is_empty()
							|| option.channel_types.contains(&target.kind))
				})
				.take(CANDIDATES)
				.map(|target| (format!("#{}", channel_label(target)), target.id.to_string())),
		);
	}
	if matches!(option.kind, 8 | 9)
		&& let Some(guild) = guild
	{
		out.extend(
			state
				.guild_roles(guild)
				.unwrap_or_default()
				.iter()
				.take(CANDIDATES)
				.map(|role| {
					let name = if role.id == guild {
						"@everyone".to_owned()
					} else {
						format!("@{}", role.name)
					};
					(name, role.id.to_string())
				}),
		);
	}
	out
}

pub enum Control {
	Text(Entity<Input>),
	/// Chosen value and its label (choices, booleans, users, channels and roles).
	Choice {
		value: String,
		label: String,
	},
	/// Attachment options cannot be filled yet.
	Unsupported,
}

pub struct Field {
	pub option: CommandOption,
	pub control: Control,
}
impl Field {
	fn value(&self, cx: &App) -> String {
		match &self.control {
			Control::Text(input) => {
				let text = input.read(cx).value();
				if matches!(self.option.kind, 4 | 10) {
					text.trim().to_owned()
				} else {
					text.to_owned()
				}
			}
			Control::Choice { value, .. } => value.clone(),
			Control::Unsupported => String::new(),
		}
	}
	fn problem(&self, attempted: bool, cx: &App) -> Option<String> {
		let value = self.value(cx);
		if self.option.kind == 11 && self.option.required {
			return Some("Attachment options are not supported yet".into());
		}
		if value.is_empty() {
			return (attempted && self.option.required)
				.then(|| format!("{} is required", self.option.name));
		}
		self.option.problem(&value)
	}
}

/// The chosen command and its option fields; the composer holds `/name `.
pub struct Active {
	pub id: Id,
	pub path: Vec<String>,
	pub name: String,
	pub description: String,
	pub application: String,
	pub fields: Vec<Field>,
	pub focused: usize,
	/// Field whose chooser is open.
	pub open: Option<usize>,
	/// A submission was refused, so empty required fields show as problems.
	pub attempted: bool,
	_subscriptions: Vec<Subscription>,
}

pub struct Picker {
	pub entries: Vec<Entry>,
	pub selected: usize,
	loading: bool,
	error: Option<&'static str>,
}

#[derive(Default)]
pub struct Slash {
	channel: Option<Id>,
	/// Search after the leading `/`, while the composer holds one.
	query: Option<String>,
	/// Query at which Escape closed the picker; editing reopens it.
	dismissed: Option<String>,
	/// Catalog request, loading flag and size when the picker was built.
	stamp: (u64, bool, usize),
	pub picker: Option<Picker>,
	pub active: Option<Active>,
	/// Last interaction and catalog errors shown as notices.
	seen: (Option<&'static str>, Option<&'static str>),
}
impl Slash {
	/// Keyboard Up/Down/Enter/Escape belong to the picker.
	pub fn picking(&self) -> bool {
		self.picker.as_ref().is_some_and(|p| !p.entries.is_empty())
	}
}

fn stamp(state: &State) -> (u64, bool, usize) {
	let catalog = &state.application_commands;
	(catalog.request, catalog.loading, catalog.commands.len())
}

/// Placeholder for a typed option.
fn hint(option: &CommandOption) -> &'static str {
	match option.kind {
		4 => "whole number",
		10 => "number",
		_ => "text",
	}
}

impl Serein {
	/// Tracks `/` in the composer after an edit; true while it is in command mode, so the
	/// `@`/`#`/`:` suggestions stay closed.
	pub(crate) fn update_slash(&mut self, cx: &mut Context<Self>) -> bool {
		let channel = self.state.selected;
		if self.slash.channel != channel {
			self.slash = Slash {
				channel,
				..Slash::default()
			};
		}
		let Some(channel) = channel else {
			return false;
		};
		let text = self.composer.read(cx).value().to_owned();
		if let Some(active) = &self.slash.active {
			if text.trim_end() == format!("/{}", active.name) {
				return true;
			}
			self.slash.active = None;
		}
		self.slash.query = query(&text);
		let Some(query) = self.slash.query.clone() else {
			self.slash.picker = None;
			self.slash.dismissed = None;
			return false;
		};
		if self.slash.dismissed.as_ref() == Some(&query) {
			self.slash.picker = None;
			return true;
		}
		self.slash.dismissed = None;
		let command = session(&mut self.state, |state| {
			state.request_application_commands(channel, false)
		});
		self.dispatch(command);
		self.rebuild_slash(channel, &query);
		true
	}

	fn rebuild_slash(&mut self, channel: Id, query: &str) {
		let selected = self.slash.picker.take().map_or(0, |p| p.selected);
		let entries = session(&mut self.state, |state| entries(state, channel, query));
		let covered = self.state.application_commands_cover(channel);
		let catalog = &self.state.application_commands;
		let loading = covered && catalog.loading;
		let error = catalog.error.filter(|_| covered);
		self.slash.stamp = stamp(&self.state);
		// Hide the picker for text that matches nothing, like `/shrug` without such a command.
		if entries.is_empty() && (query.contains(' ') || !(loading || error.is_some())) {
			return;
		}
		self.slash.picker = Some(Picker {
			selected: selected.min(entries.len().saturating_sub(1)),
			entries,
			loading,
			error,
		});
	}

	/// Keys routed from the composer while the picker is open; false when it is closed.
	pub(crate) fn slash_pick(
		&mut self,
		key: input::Pick,
		window: &mut Window,
		cx: &mut Context<Self>,
	) -> bool {
		let Some(picker) = self.slash.picker.as_mut().filter(|p| !p.entries.is_empty()) else {
			return false;
		};
		let count = picker.entries.len();
		match key {
			input::Pick::Up => picker.selected = (picker.selected + count - 1) % count,
			input::Pick::Down => picker.selected = (picker.selected + 1) % count,
			input::Pick::Accept => {
				let index = picker.selected;
				self.accept_command(index, window, cx);
			}
			input::Pick::Close => {
				self.slash.dismissed = self.slash.query.clone();
				self.slash.picker = None;
				self.composer
					.update(cx, |input, _| input.set_picking(false));
			}
		}
		cx.notify();
		true
	}

	/// Chooses a picker entry: the composer shows `/name ` and its options become fields.
	pub(crate) fn accept_command(
		&mut self,
		index: usize,
		window: &mut Window,
		cx: &mut Context<Self>,
	) {
		let Some(entry) = self
			.slash
			.picker
			.take()
			.and_then(|picker| picker.entries.into_iter().nth(index))
		else {
			return;
		};
		let Some(command) = self
			.state
			.application_commands
			.commands
			.iter()
			.find(|command| command.id == entry.id)
		else {
			return;
		};
		let options = command
			.options_at(&entry.path)
			.map(<[_]>::to_vec)
			.unwrap_or_default();
		let mut fields = Vec::with_capacity(options.len());
		let mut subscriptions = Vec::new();
		for (index, option) in options.into_iter().enumerate() {
			let control = if option.kind == 11 {
				Control::Unsupported
			} else if option.kind == 5
				|| !option.choices.is_empty()
				|| (6..=9).contains(&option.kind)
			{
				Control::Choice {
					value: String::new(),
					label: String::new(),
				}
			} else {
				let input = cx.new(Input::new);
				input.update(cx, |input, cx| {
					input.set_placeholder(hint(&option).into(), cx)
				});
				subscriptions.push(cx.subscribe_in(
					&input,
					window,
					|this, _, _: &input::Submit, window, cx| {
						if this.run_slash(cx) && this.slash.active.is_none() {
							let focus = this.composer.read(cx).focus_handle(cx);
							window.focus(&focus, cx);
						}
					},
				));
				subscriptions.push(cx.subscribe_in(
					&input,
					window,
					move |this, _, event: &input::Event, window, cx| {
						this.slash_field_event(index, event, window, cx)
					},
				));
				Control::Text(input)
			};
			fields.push(Field { option, control });
		}
		let first = fields.iter().find_map(|field| match &field.control {
			Control::Text(input) => Some(input.read(cx).focus_handle(cx)),
			_ => None,
		});
		self.slash.active = Some(Active {
			id: entry.id,
			path: entry.path,
			name: entry.name.clone(),
			description: entry.description,
			application: entry.application,
			fields,
			focused: 0,
			open: None,
			attempted: false,
			_subscriptions: subscriptions,
		});
		self.composer.update(cx, |input, cx| {
			input.set_picking(false);
			input.set_value(format!("/{} ", entry.name), cx);
		});
		if let Some(focus) = first {
			window.focus(&focus, cx);
		}
		cx.notify();
	}

	fn slash_field_event(
		&mut self,
		index: usize,
		event: &input::Event,
		window: &mut Window,
		cx: &mut Context<Self>,
	) {
		let Some(active) = &mut self.slash.active else {
			return;
		};
		match event {
			input::Event::Changed => {
				active.focused = index;
				active.open = None;
			}
			input::Event::Cancel => {
				if active.open.take().is_none() {
					let focus = self.composer.read(cx).focus_handle(cx);
					window.focus(&focus, cx);
				}
			}
			input::Event::EditLast | input::Event::Pick(_) => return,
		}
		cx.notify();
	}

	/// Escape in the composer drops the chosen command; false when none is chosen.
	pub(crate) fn cancel_slash(&mut self, cx: &mut Context<Self>) -> bool {
		if self.slash.active.take().is_none() {
			return false;
		}
		if let Some(channel) = self.state.selected {
			self.state.drafts.remove(&channel);
		}
		self.slash.query = None;
		self.composer
			.update(cx, |input, cx| input.set_value(String::new(), cx));
		cx.notify();
		true
	}

	/// Enter with a chosen command runs it through the reducer; true when the key was used.
	pub(crate) fn run_slash(&mut self, cx: &mut Context<Self>) -> bool {
		let Some(channel) = self.state.selected else {
			return false;
		};
		let Some(active) = &self.slash.active else {
			// A typed `/name` from the index must be chosen from the list first.
			let text = self.composer.read(cx).value();
			let name = text
				.strip_prefix('/')
				.and_then(|rest| rest.split_whitespace().next());
			let listed = self.state.application_commands_cover(channel)
				&& name.is_some_and(|name| {
					self.state
						.application_commands
						.commands
						.iter()
						.any(|command| command.name == name)
				});
			if listed {
				self.notify_user("Choose a command from the list before running it.");
				cx.notify();
			}
			return listed;
		};
		let values = active
			.fields
			.iter()
			.map(|field| (field.option.name.clone(), field.value(cx)))
			.collect::<Vec<_>>();
		let (id, path) = (active.id, active.path.clone());
		let result = session(&mut self.state, |state| {
			state.prepare_application_command(id, &path, &values)
		});
		match result {
			Ok(command) => {
				self.dispatch(Some(command));
				self.slash.active = None;
				self.slash.query = None;
				self.state.drafts.remove(&channel);
				self.composer.update(cx, |input, cx| {
					input.set_picking(false);
					input.set_value(String::new(), cx);
				});
				// A refusal that arrived synchronously (queue full, offline) is a notice now.
				if let Some(error) = self.state.interactions.error {
					self.slash.seen.0 = Some(error);
					self.notify_user(error);
				}
			}
			Err(error) => {
				if let Some(active) = &mut self.slash.active {
					active.attempted = true;
				}
				self.notify_user(error);
			}
		}
		cx.notify();
		true
	}

	/// Per-poll upkeep: channel changes, interaction timeouts, a catalog that arrived while
	/// the picker was open, and reducer errors as transient notices. True when redrawing.
	pub(crate) fn poll_slash(&mut self, cx: &mut Context<Self>) -> bool {
		let mut changed = false;
		if self.slash.channel != self.state.selected
			&& (self.slash.query.is_some() || self.slash.active.is_some())
		{
			self.slash = Slash {
				channel: self.state.selected,
				seen: self.slash.seen,
				..Slash::default()
			};
			self.composer
				.update(cx, |input, _| input.set_picking(false));
			changed = true;
		}
		session(&mut self.state, |state| {
			state.expire_interaction(Instant::now())
		});
		if self.slash.active.is_none()
			&& self.slash.dismissed.is_none()
			&& let (Some(channel), Some(query)) = (self.state.selected, self.slash.query.clone())
			&& self.slash.stamp != stamp(&self.state)
		{
			self.rebuild_slash(channel, &query);
			let picking = self.slash.picking();
			self.composer
				.update(cx, |input, _| input.set_picking(picking));
			changed = true;
		}
		let errors = (
			self.state.interactions.error,
			self.state.application_commands.error,
		);
		if errors != self.slash.seen {
			let (interaction, catalog) = std::mem::replace(&mut self.slash.seen, errors);
			if let Some(error) = errors.0.filter(|e| Some(*e) != interaction) {
				self.notify_user(error);
			}
			if let Some(error) = errors.1.filter(|e| Some(*e) != catalog) {
				self.notify_user(error);
			}
			changed = true;
		}
		changed
	}

	/// Screenshot state: /weather chosen from the open picker with filled options; `run`
	/// submits valid values, otherwise `days` is out of range to show validation.
	pub(crate) fn demo_slash_options(
		&mut self,
		run: bool,
		window: &mut Window,
		cx: &mut Context<Self>,
	) {
		let Some(index) = self
			.slash
			.picker
			.as_ref()
			.and_then(|picker| picker.entries.iter().position(|e| e.name == "weather"))
		else {
			return;
		};
		self.accept_command(index, window, cx);
		let Some(active) = &mut self.slash.active else {
			return;
		};
		for (index, field) in active.fields.iter_mut().enumerate() {
			match (field.option.name.as_str(), &mut field.control) {
				("city", Control::Text(input)) => {
					input.update(cx, |input, cx| input.set_value("Prague".into(), cx))
				}
				("days", Control::Text(input)) => {
					let days = if run { "3" } else { "9" };
					input.update(cx, |input, cx| input.set_value(days.into(), cx));
					active.focused = index;
				}
				("units", Control::Choice { value, label }) => {
					*value = "celsius".into();
					*label = "Celsius".into();
				}
				_ => {}
			}
		}
		if run {
			self.run_slash(cx);
		}
	}

	/// Sets a chooser field's value.
	fn choose(&mut self, index: usize, value: String, label: String, cx: &mut Context<Self>) {
		if let Some(active) = &mut self.slash.active {
			if let Some(Field {
				control: Control::Choice { value: v, label: l },
				..
			}) = active.fields.get_mut(index)
			{
				*v = value;
				*l = label;
			}
			active.open = None;
			active.focused = index;
		}
		cx.notify();
	}

	pub(crate) fn render_slash_picker(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
		let picker = self.slash.picker.as_ref()?;
		let p = palette();
		let start = picker
			.selected
			.saturating_sub(ROWS - 1)
			.min(picker.entries.len().saturating_sub(ROWS));
		let mut list = div().flex().flex_col();
		let mut previous: Option<Id> = None;
		for (index, entry) in picker.entries.iter().enumerate().skip(start).take(ROWS) {
			if previous != Some(entry.application_id) {
				previous = Some(entry.application_id);
				list = list.child(
					div()
						.px_2()
						.pt(px(if index == start { 2. } else { 8. }))
						.pb_1()
						.flex()
						.items_center()
						.gap_2()
						.child(avatar(&entry.application, 16., None))
						.child(
							div()
								.text_size(px(12.))
								.font_weight(FontWeight::SEMIBOLD)
								.text_color(color(p.muted))
								.child(entry.application.to_uppercase()),
						),
				);
			}
			let selected = index == picker.selected;
			let options = self
				.state
				.application_commands
				.commands
				.iter()
				.find(|command| command.id == entry.id)
				.and_then(|command| command.options_at(&entry.path).ok())
				.map(|options| {
					options
						.iter()
						.map(|option| option.name.as_str())
						.collect::<Vec<_>>()
						.join("  ")
				})
				.unwrap_or_default();
			list = list.child(
				div()
					.id(("slash-command", index))
					.h(px(48.))
					.px_2()
					.rounded(px(6.))
					.flex()
					.items_center()
					.gap_3()
					.cursor_pointer()
					.when(selected, |d| d.bg(color(p.selected)))
					.hover(|d| d.bg(color(p.hover)))
					.on_click(cx.listener(move |this, _, window, cx| {
						this.accept_command(index, window, cx)
					}))
					.child(
						div()
							.flex_1()
							.min_w_0()
							.flex()
							.flex_col()
							.child(
								div()
									.flex()
									.items_baseline()
									.gap_2()
									.whitespace_nowrap()
									.overflow_hidden()
									.child(
										div()
											.flex_none()
											.text_size(px(15.))
											.font_weight(FontWeight::SEMIBOLD)
											.text_color(color(p.text_strong))
											.child(format!("/{}", entry.name)),
									)
									.child(
										div()
											.min_w_0()
											.text_size(px(13.))
											.text_color(color(p.muted))
											.text_ellipsis()
											.child(options),
									),
							)
							.child(
								div()
									.text_size(px(13.))
									.text_color(color(p.muted))
									.whitespace_nowrap()
									.overflow_hidden()
									.text_ellipsis()
									.child(entry.description.clone()),
							),
					)
					.child(
						div()
							.flex_none()
							.text_size(px(12.))
							.text_color(color(p.muted))
							.child(entry.application.clone()),
					),
			);
		}
		let status = if picker.loading {
			Some(("Loading commands…", p.muted))
		} else {
			picker.error.map(|error| (error, p.danger))
		};
		Some(
			div()
				.absolute()
				.left(px(16.))
				.right(px(16.))
				.bottom_full()
				.mb_1()
				.p_2()
				.rounded(px(8.))
				.bg(color(p.base))
				.border_1()
				.border_color(color(p.border))
				.shadow_lg()
				.flex()
				.flex_col()
				.child(list)
				.children(status.map(|(text, tone)| {
					div()
						.px_2()
						.py_2()
						.text_size(px(13.))
						.text_color(color(tone))
						.child(text)
				}))
				.into_any_element(),
		)
	}

	/// Option chips, the focused option's help or problem, and an open chooser.
	pub(crate) fn render_slash_options(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
		let active = self.slash.active.as_ref()?;
		let channel = self.state.selected?;
		let p = palette();
		let value_fill = color(if crate::theme::dark() { p.base } else { p.chat });
		let problems = active
			.fields
			.iter()
			.map(|field| field.problem(active.attempted, cx))
			.collect::<Vec<_>>();
		let mut chips = div().flex().flex_wrap().gap_2();
		for (index, field) in active.fields.iter().enumerate() {
			let invalid = problems[index].is_some();
			let focused = index == active.focused;
			let option = &field.option;
			let label = div()
				.h_full()
				.px(px(10.))
				.flex()
				.items_center()
				.gap(px(2.))
				.bg(color(p.hover))
				.text_size(px(14.))
				.font_weight(FontWeight::MEDIUM)
				.text_color(color(if option.required { p.text } else { p.muted }))
				.child(option.name.clone())
				.when(option.required, |d| {
					d.child(div().text_color(color(p.danger)).child("*"))
				});
			let value = match &field.control {
				Control::Text(input) => div()
					.w(px(if matches!(option.kind, 4 | 10) {
						96.
					} else {
						180.
					}))
					.px_2()
					.child(input.clone())
					.into_any_element(),
				Control::Choice { label, .. } => div()
					.id(("slash-choose", index))
					.h_full()
					.px_2()
					.flex()
					.items_center()
					.gap_1()
					.cursor_pointer()
					.hover(|d| d.bg(color(p.hover)))
					.on_click(cx.listener(move |this, _, _, cx| {
						if let Some(active) = &mut this.slash.active {
							active.open = (active.open != Some(index)).then_some(index);
							active.focused = index;
						}
						cx.notify();
					}))
					.child(
						div()
							.max_w(px(200.))
							.whitespace_nowrap()
							.overflow_hidden()
							.text_ellipsis()
							.text_size(px(14.))
							.text_color(color(if label.is_empty() {
								p.muted
							} else {
								p.text_strong
							}))
							.child(if label.is_empty() {
								"Choose…".to_owned()
							} else {
								label.clone()
							}),
					)
					.child(icon(Icon::CaretDown, px(12.), color(p.muted)))
					.into_any_element(),
				Control::Unsupported => div()
					.px_2()
					.text_size(px(13.))
					.text_color(color(p.muted))
					.child("Unavailable")
					.into_any_element(),
			};
			chips = chips.child(
				div()
					.id(("slash-option", index))
					.h(px(32.))
					.rounded(px(6.))
					.overflow_hidden()
					.bg(value_fill)
					.border_1()
					.border_color(if invalid {
						color(p.danger)
					} else if focused {
						color(p.muted)
					} else {
						color(p.border)
					})
					.flex()
					.items_center()
					.tooltip(crate::tooltip(option.description.clone()))
					.on_click(cx.listener(move |this, _, _, cx| {
						if let Some(active) = &mut this.slash.active {
							active.focused = index;
						}
						cx.notify();
					}))
					.child(label)
					.child(value),
			);
		}
		// One help line: the focused option's problem, another option's problem, the
		// reducer's error, waiting, then the focused option's description.
		let option = active.fields.get(active.focused).map(|field| &field.option);
		let problem = problems
			.get(active.focused)
			.cloned()
			.flatten()
			.or_else(|| problems.iter().flatten().next().cloned());
		let busy = self.state.interactions.busy();
		let (title, detail, tone) = if let Some(problem) = problem {
			let name = active
				.fields
				.iter()
				.zip(&problems)
				.find(|(_, p)| p.as_ref() == Some(&problem))
				.map_or(active.name.clone(), |(field, _)| field.option.name.clone());
			(name, problem, p.danger)
		} else if let Some(error) = self.state.interactions.error {
			(active.name.clone(), error.to_owned(), p.danger)
		} else if busy {
			(
				active.name.clone(),
				"Waiting for the application…".to_owned(),
				p.muted,
			)
		} else if let Some(option) = option {
			let suffix = if option.required { "" } else { " · Optional" };
			(
				option.name.clone(),
				format!("{}{suffix}", option.description),
				p.muted,
			)
		} else {
			(
				active.name.clone(),
				format!("{} · Press Enter to run", active.description),
				p.muted,
			)
		};
		let chooser = active.open.and_then(|index| {
			let field = active.fields.get(index)?;
			let mut rows = vec![("Not set".to_owned(), String::new())];
			rows.extend(candidates(&self.state, channel, &field.option));
			Some(
				div()
					.id("slash-chooser")
					.absolute()
					.left(px(12.))
					.bottom_full()
					.mb_1()
					.w(px(280.))
					.max_h(px(280.))
					.overflow_y_scroll()
					.p_1()
					.rounded(px(8.))
					.bg(color(p.base))
					.border_1()
					.border_color(color(p.border))
					.shadow_lg()
					.children(rows.into_iter().enumerate().map(|(row, (label, value))| {
						let shown = if value.is_empty() {
							String::new()
						} else {
							label.clone()
						};
						div()
							.id(("slash-choice", row))
							.h(px(32.))
							.px_2()
							.rounded(px(4.))
							.flex()
							.items_center()
							.cursor_pointer()
							.hover(|d| d.bg(color(p.hover)))
							.text_size(px(14.))
							.text_color(color(if value.is_empty() { p.muted } else { p.text }))
							.whitespace_nowrap()
							.overflow_hidden()
							.text_ellipsis()
							.on_click(cx.listener(move |this, _, _, cx| {
								this.choose(index, value.clone(), shown.clone(), cx)
							}))
							.child(label)
					})),
			)
		});
		Some(
			div()
				.relative()
				.px_3()
				.pt_2()
				.pb(px(6.))
				.rounded_t(px(8.))
				.bg(color(ui::design::mix(p.raised, p.base, 0.45)))
				.flex()
				.flex_col()
				.gap_2()
				.child(
					div()
						.flex()
						.items_center()
						.gap_2()
						.child(avatar(&active.application, 20., None))
						.child(
							div()
								.flex_none()
								.text_size(px(14.))
								.font_weight(FontWeight::SEMIBOLD)
								.text_color(color(p.text_strong))
								.child(format!("/{}", active.name)),
						)
						.child(
							div()
								.flex_1()
								.min_w_0()
								.text_size(px(13.))
								.text_color(color(p.muted))
								.whitespace_nowrap()
								.overflow_hidden()
								.text_ellipsis()
								.child(active.application.clone()),
						)
						.child(
							self.icon_button("slash-cancel", Icon::Close, false, "Cancel command")
								.size(px(22.))
								.on_click(cx.listener(|this, _, _, cx| {
									this.cancel_slash(cx);
								})),
						),
				)
				.when(!active.fields.is_empty(), |d| d.child(chips))
				.child(
					div()
						.flex()
						.gap_2()
						.text_size(px(13.))
						.whitespace_nowrap()
						.overflow_hidden()
						.child(
							div()
								.flex_none()
								.font_weight(FontWeight::SEMIBOLD)
								.text_color(color(p.text_strong))
								.child(title),
						)
						.child(
							div()
								.min_w_0()
								.text_ellipsis()
								.text_color(color(tone))
								.child(detail),
						),
				)
				.children(chooser)
				.into_any_element(),
		)
	}

	/// Private replies for this conversation, newest last, each with a Dismiss link.
	pub(crate) fn render_ephemeral(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
		let channel = self.state.selected?;
		let replies = self
			.state
			.interactions
			.ephemeral
			.iter()
			.filter(|message| message.channel == channel)
			.collect::<Vec<_>>();
		if replies.is_empty() {
			return None;
		}
		let p = palette();
		let skip = replies.len().saturating_sub(REPLIES);
		Some(
			div()
				.flex()
				.flex_col()
				.gap_1()
				.pb_2()
				.children(replies.into_iter().skip(skip).map(|message| {
					let id = message.id;
					let name = self.state.message_author_name(message).to_owned();
					div()
						.mx_4()
						.px_3()
						.py_2()
						.rounded(px(8.))
						.bg(tint(p.accent, 0.08))
						.border_l_2()
						.border_color(color(p.accent))
						.flex()
						.gap_3()
						.child(avatar(&name, 32., Some(&message.author)))
						.child(
							div()
								.flex_1()
								.min_w_0()
								.flex()
								.flex_col()
								.gap(px(2.))
								.children(message.interaction.as_ref().map(|used| {
									div().text_size(px(12.)).text_color(color(p.muted)).child(
										format!(
											"{} used /{}",
											self.state.user_display_name(&used.user),
											used.command
										),
									)
								}))
								.child(
									div()
										.flex()
										.items_center()
										.gap_2()
										.child(
											div()
												.text_size(px(15.))
												.font_weight(FontWeight::MEDIUM)
												.text_color(color(p.text_strong))
												.child(name),
										)
										.children(message.author.account_label().map(|label| {
											div()
												.h(px(16.))
												.px(px(4.))
												.rounded(px(3.))
												.bg(color(p.accent))
												.flex()
												.items_center()
												.text_size(px(10.))
												.font_weight(FontWeight::SEMIBOLD)
												.text_color(color(p.accent_text))
												.child(label)
										})),
								)
								.child(reply_text(&message.display_text()))
								.child(
									div()
										.flex()
										.items_center()
										.gap_1()
										.text_size(px(12.))
										.text_color(color(p.muted))
										.child("Only you can see this ·")
										.child(
											div()
												.id(("dismiss-ephemeral", id.0))
												.cursor_pointer()
												.text_color(color(p.link))
												.hover(|d| d.underline())
												.on_click(cx.listener(move |this, _, _, cx| {
													this.state.dismiss_ephemeral(id);
													cx.notify();
												}))
												.child("Dismiss message"),
										),
								),
						)
				}))
				.into_any_element(),
		)
	}
}

/// Private reply text with basic Markdown (bold, italic, code, strikethrough).
fn reply_text(source: &str) -> AnyElement {
	let p = palette();
	let source: String = source.chars().take(2000).collect();
	let formatted = ui::Formatted::parse(&source);
	let mut text = String::new();
	let mut runs = Vec::new();
	for span in formatted.spans() {
		if span.text.is_empty() {
			continue;
		}
		text.push_str(span.text);
		let mut font = font(if span.code || span.block.is_some() {
			MONO
		} else {
			FONT
		});
		if span.strong || span.heading > 0 {
			font.weight = FontWeight::SEMIBOLD;
		}
		if span.italic {
			font.style = FontStyle::Italic;
		}
		let tone: Hsla = color(if span.small { p.muted } else { p.text }).into();
		runs.push(TextRun {
			len: span.text.len(),
			font,
			color: tone,
			background_color: (span.code || span.block.is_some())
				.then(|| tint(p.raised, 1.).into()),
			underline: None,
			strikethrough: span.strike.then(|| StrikethroughStyle {
				thickness: px(1.),
				color: Some(tone),
			}),
		});
	}
	div()
		.text_size(px(15.))
		.line_height(px(22.))
		.child(StyledText::new(text).with_runs(runs))
		.into_any_element()
}

// Synthetic offline catalog and replies: nothing here contacts an application.

fn option(kind: u8, name: &str, description: &str) -> CommandOption {
	CommandOption {
		kind,
		name: name.into(),
		description: description.into(),
		..CommandOption::default()
	}
}

fn choices(pairs: &[(&str, &str)]) -> Vec<schema::Choice> {
	pairs
		.iter()
		.map(|(name, value)| schema::Choice {
			name: (*name).into(),
			value: Value::String((*value).into()),
		})
		.collect()
}

/// The desktop demo's synthetic apps and commands, plus one with user/channel/role options.
pub fn demo_catalog(guild: Option<Id>) -> Vec<schema::Command> {
	let command = |id: u64, application: u64, name: &str, description: &str, options| {
		schema::Command {
			id: Id(id),
			version: Id(1),
			application_id: Id(application),
			guild_id: guild,
			kind: 1,
			name: name.into(),
			description: description.into(),
			options,
			contexts: Some(vec![0, 1]),
			integration_types: None,
			application_name: if application == 99000 {
				"Atlas (synthetic)"
			} else {
				"Studio (synthetic)"
			}
			.into(),
			// Never fetch third-party app icons offline; the picker shows initials.
			application_icon: None,
			default_member_permissions: None,
			permissions: Default::default(),
			application_permissions: Default::default(),
		}
	};
	let weather = vec![
		CommandOption {
			required: true,
			max_length: Some(100),
			..option(3, "city", "City to look up.")
		},
		CommandOption {
			choices: choices(&[("Celsius", "celsius"), ("Fahrenheit", "fahrenheit")]),
			..option(3, "units", "Temperature units.")
		},
		CommandOption {
			min_value: Some(1.),
			max_value: Some(7.),
			..option(4, "days", "Number of forecast days.")
		},
		option(5, "detailed", "Include a detailed forecast."),
	];
	let remind = vec![
		CommandOption {
			required: true,
			..option(6, "who", "Who to remind.")
		},
		CommandOption {
			required: true,
			max_length: Some(200),
			..option(3, "note", "What to remind them about.")
		},
		CommandOption {
			channel_types: vec![0],
			..option(7, "where", "Channel for the reminder.")
		},
		option(8, "notify", "Role to notify as well."),
	];
	let canvas = vec![CommandOption {
		options: vec![CommandOption {
			options: vec![
				CommandOption {
					required: true,
					max_length: Some(500),
					..option(3, "prompt", "Describe the image.")
				},
				CommandOption {
					choices: choices(&[("Watercolor", "watercolor"), ("Pixel art", "pixel")]),
					..option(3, "style", "Visual style.")
				},
				CommandOption {
					min_value: Some(0.5),
					max_value: Some(2.),
					..option(10, "scale", "Output scale, from 0.5 to 2.")
				},
			],
			..option(1, "create", "Create an image from a prompt.")
		}],
		..option(2, "image", "Image tools.")
	}];
	let commands = vec![
		command(99101, 99000, "weather", "Look up a city forecast.", weather),
		command(
			99102,
			99000,
			"ping",
			"Check whether the app is responding.",
			vec![],
		),
		command(
			99104,
			99000,
			"help",
			"Get help with the app's commands.",
			vec![CommandOption {
				max_length: Some(100),
				..option(3, "input", "The command or topic to learn about.")
			}],
		),
		command(
			99105,
			99000,
			"remind",
			"Set a reminder for someone.",
			remind,
		),
		command(
			99103,
			99010,
			"canvas",
			"Create something with Studio.",
			canvas,
		),
	];
	debug_assert!(schema::valid_catalog(&commands));
	commands
}

/// Lets the demo's synthetic member use application commands (the everyone role gains
/// `USE_APPLICATION_COMMANDS`); other permissions are untouched.
pub fn demo_permissions(state: &mut State) {
	let roles = state
		.guilds
		.iter()
		.filter_map(|guild| {
			let role = state
				.guild_roles(guild.id)?
				.iter()
				.find(|r| r.id == guild.id)?;
			Some((guild.id, role.clone()))
		})
		.collect::<Vec<_>>();
	for (guild, mut role) in roles {
		role.bits |= model::permissions::USE_APPLICATION_COMMANDS;
		state.apply(Envelope {
			generation: state.generation,
			event: Event::Permissions(client_core::permissions::Event::Role { guild, role }),
		});
	}
}

fn describe(arguments: &[schema::Argument], out: &mut Vec<String>) {
	for argument in arguments {
		match &argument.value {
			Some(value) => out.push(format!("{}: {}", argument.name, value_text(value))),
			None => describe(&argument.options, out),
		}
	}
}

/// Offline answers: the synthetic catalog, and a private reply to each invocation.
pub fn demo_respond(state: &mut State, command: Command) {
	session(state, |state| {
		let event = match command {
			Command::ApplicationCommands {
				channel,
				guild,
				request,
			} => Event::ApplicationCommands {
				channel,
				request,
				result: Ok(demo_catalog(guild)),
			},
			Command::Interaction(request) => {
				let interactions::Data::ApplicationCommand { invocation } = &request.data else {
					state.apply(Envelope {
						generation: state.generation,
						event: Event::Interaction(interactions::Event::Submitted {
							nonce: request.nonce,
							result: Err(client_core::auth::Failure::ProtocolAt(
								"Offline preview does not contact applications",
							)),
						}),
					});
					return;
				};
				let mut arguments = Vec::new();
				describe(&invocation.options, &mut arguments);
				let path = invocation
					.options
					.first()
					.filter(|o| o.value.is_none())
					.map(|o| {
						let mut path = format!(" {}", o.name);
						if let Some(sub) = o.options.first().filter(|s| s.value.is_none()) {
							path.push(' ');
							path.push_str(&sub.name);
						}
						path
					})
					.unwrap_or_default();
				let name = format!("{}{path}", invocation.command.name);
				let millis = std::time::SystemTime::now()
					.duration_since(std::time::UNIX_EPOCH)
					.map_or(0, |d| d.as_millis() as u64);
				let mut reply = test_support::message(
					(millis.saturating_sub(1_420_070_400_000) << 22) | (request.request & 0xfff),
					request.channel_id,
				);
				reply.author = model::User {
					id: request.application_id,
					name: invocation.command.application_name.clone(),
					kind: model::AccountKind::Bot,
					webhook: false,
					avatar: None,
					discriminator: 0,
					primary_guild: None,
				};
				reply.author_roles = Vec::new();
				reply.author_nick = None;
				reply.mentions = Vec::new();
				reply.reactions = Some(Vec::new());
				reply.embeds = Vec::new();
				reply.attachments = Vec::new();
				reply.application_id = Some(request.application_id);
				reply.flags = 64;
				reply.ephemeral = true;
				reply.interaction = state.user.clone().map(|user| {
					Box::new(model::Interaction {
						user,
						command: name.clone(),
					})
				});
				reply.content = format!(
					"**Synthetic /{name} response**\nArguments: {}\nNo application was contacted.",
					if arguments.is_empty() {
						"none".to_owned()
					} else {
						arguments.join(" · ")
					}
				);
				state.apply(Envelope {
					generation: state.generation,
					event: Event::Interaction(interactions::Event::Success {
						nonce: request.nonce,
					}),
				});
				Event::Interaction(interactions::Event::Ephemeral(Box::new(reply)))
			}
			other => {
				state.command_rejected(other);
				return;
			}
		};
		state.apply(Envelope {
			generation: state.generation,
			event,
		});
	});
}

#[cfg(test)]
mod tests {
	use super::{demo_permissions, demo_respond, entries, query, session};
	use client_core::{Command, State};
	use model::Id;

	fn loaded() -> (State, Id) {
		let mut state = test_support::chat_demo_state();
		demo_permissions(&mut state);
		let channel = state.selected.expect("demo channel");
		let command = session(&mut state, |state| {
			state.request_application_commands(channel, false)
		})
		.expect("catalog request");
		demo_respond(&mut state, command);
		(state, channel)
	}

	#[test]
	fn query_is_the_text_after_a_leading_slash() {
		assert_eq!(query("/"), Some(String::new()));
		assert_eq!(query("/Wea "), Some("wea".into()));
		assert_eq!(query("hello /wea"), None);
		assert_eq!(query("/a\nb"), None);
		assert_eq!(query(&format!("/{}", "a".repeat(129))), None);
	}

	#[test]
	fn demo_catalog_loads_offline_and_groups_by_application() {
		let (mut state, channel) = loaded();
		let auth = state.auth;
		let all = session(&mut state, |state| entries(state, channel, ""));
		assert_eq!(state.auth, auth, "session flags are restored");
		let names = all.iter().map(|e| e.name.as_str()).collect::<Vec<_>>();
		assert_eq!(
			names,
			["help", "ping", "remind", "weather", "canvas image create"]
		);
		assert_eq!(all[4].path, ["image", "create"]);
		assert_eq!(all[4].application, "Studio (synthetic)");
		let forecast = session(&mut state, |state| entries(state, channel, "forecast"));
		assert_eq!(forecast.len(), 1);
		assert_eq!(forecast[0].name, "weather");
		assert!(session(&mut state, |state| entries(state, channel, "zzz")).is_empty());
		// Without the synthetic permission the index is not offered.
		let mut plain = test_support::chat_demo_state();
		assert!(
			session(&mut plain, |s| s
				.request_application_commands(channel, false))
			.is_none()
		);
	}

	#[test]
	fn invocations_validate_and_reply_privately() {
		let (mut state, _) = loaded();
		let missing = session(&mut state, |state| {
			state.prepare_application_command(Id(99101), &[], &[])
		});
		assert!(missing.is_err());
		let out_of_range = session(&mut state, |state| {
			state.prepare_application_command(
				Id(99101),
				&[],
				&[
					("city".into(), "Prague".into()),
					("days".into(), "9".into()),
				],
			)
		});
		assert!(out_of_range.is_err());
		assert!(!state.interactions.busy());
		let values = [
			("city".into(), "Prague".into()),
			("units".into(), "celsius".into()),
			("days".into(), "3".into()),
			("detailed".into(), String::new()),
		];
		let command = session(&mut state, |state| {
			state.prepare_application_command(Id(99101), &[], &values)
		})
		.expect("valid invocation");
		assert!(matches!(command, Command::Interaction(_)));
		demo_respond(&mut state, command);
		assert!(!state.interactions.busy());
		let reply = state.interactions.ephemeral.last().expect("private reply");
		assert!(reply.ephemeral);
		assert!(
			reply
				.content
				.contains("city: Prague · units: celsius · days: 3")
		);
		assert!(state.timeline.get(reply.id).is_none());
		let nested = session(&mut state, |state| {
			state.prepare_application_command(
				Id(99103),
				&["image".into(), "create".into()],
				&[("prompt".into(), "A moonlit forest".into())],
			)
		})
		.expect("nested invocation");
		demo_respond(&mut state, nested);
		let reply = state.interactions.ephemeral.last().unwrap();
		assert!(reply.content.contains("/canvas image create"));
		assert_eq!(state.interactions.ephemeral.len(), 2);
	}

	#[test]
	fn user_channel_and_role_options_accept_ids() {
		let (mut state, channel) = loaded();
		let remind = state
			.application_commands
			.commands
			.iter()
			.find(|c| c.name == "remind")
			.cloned()
			.unwrap();
		let pick = |kind| {
			super::candidates(
				&state,
				channel,
				remind.options.iter().find(|o| o.kind == kind).unwrap(),
			)
		};
		let (who, place, role) = (pick(6), pick(7), pick(8));
		assert!(!who.is_empty() && !place.is_empty() && !role.is_empty());
		assert!(place.iter().all(|(label, _)| label.starts_with('#')));
		assert_eq!(role[0].0, "@everyone");
		let values = [
			("who".into(), who[0].1.clone()),
			("note".into(), "Water the plants".into()),
			("where".into(), place[0].1.clone()),
			("notify".into(), role[0].1.clone()),
		];
		assert!(
			session(&mut state, |state| {
				state.prepare_application_command(remind.id, &[], &values)
			})
			.is_ok()
		);
	}
}
