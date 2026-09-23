//! Experimental native GPUI frontend; transports and secrets remain in Serein's shared crates.
mod backend;
mod chat;
mod input;
mod members;
mod sidebar;
mod signin;
mod theme;

use client_core::{Command, Envelope, Event, State};
use gpui::{prelude::*, *};
use model::Id;
use std::{
	collections::BTreeSet,
	time::{Duration, Instant},
};
use theme::{Icon, color, icon, palette};

actions!(serein, [Quit, Hide, HideOthers, ShowAll, Minimize]);

const NOTICE_TIME: Duration = Duration::from_secs(5);
/// Events applied per wakeup; a larger backlog re-arms the wakeup instead of starving input.
const EVENTS_PER_TICK: usize = 256;
const IDLE_TICK: Duration = Duration::from_millis(250);

fn tab_navigation(event: &KeyDownEvent, window: &mut Window, cx: &mut App) {
	if event.keystroke.key == "tab" {
		if event.keystroke.modifiers.shift {
			window.focus_prev(cx);
		} else {
			window.focus_next(cx);
		}
		cx.stop_propagation();
	}
}

struct Tooltip(SharedString);
impl Render for Tooltip {
	fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
		let p = palette();
		div()
			.font_family(theme::FONT)
			.px_2()
			.py_1()
			.rounded(px(6.))
			.bg(color(p.base))
			.border_1()
			.border_color(color(p.border))
			.text_sm()
			.text_color(color(p.text_strong))
			.child(self.0.clone())
	}
}
pub(crate) fn tooltip(text: impl Into<SharedString>) -> impl Fn(&mut Window, &mut App) -> AnyView {
	let text = text.into();
	move |_, cx| cx.new(|_| Tooltip(text.clone())).into()
}

pub(crate) fn channel_label(channel: &model::Channel) -> String {
	if !channel.name.is_empty() {
		channel.name.clone()
	} else {
		channel
			.recipients
			.iter()
			.map(|user| user.name.as_str())
			.collect::<Vec<_>>()
			.join(", ")
	}
}
/// Text conversations this frontend can open; voice and stage are listed but not joined.
pub(crate) fn text_channel(channel: &model::Channel) -> bool {
	channel.supports_text() && !matches!(channel.kind, 2 | 13)
}

pub(crate) struct Serein {
	state: State,
	backend: backend::Backend,
	guild: Option<Id>,
	composer: Entity<input::Input>,
	rows: Vec<Id>,
	nav: Vec<sidebar::NavRow>,
	collapsed: BTreeSet<Id>,
	members_open: bool,
	member_rows: Vec<members::MemberRow>,
	member_key: Option<members::MembersKey>,
	messages: ListState,
	format: ui::FormatCache,
	hovered: Option<Id>,
	/// Messages whose spoilers were revealed by a click; cleared on channel change.
	revealed: BTreeSet<Id>,
	/// First unread message when the channel opened; `None` until its history arrives.
	boundary: Option<Option<Id>>,
	notice: Option<(SharedString, Instant)>,
	/// Visible typists at the last redraw.
	typists: usize,
	/// Last `State::status` shown, so each new value becomes one transient notice.
	state_status: &'static str,
	status: &'static str,
	backend_status: &'static str,
	authorized: bool,
	#[cfg(not(target_os = "linux"))]
	login: Option<platform::LoginView>,
}

impl Serein {
	fn new(demo: bool, window: &mut Window, cx: &mut Context<Self>) -> Self {
		let composer = cx.new(input::Input::new);
		cx.subscribe(&composer, |this, _, _: &input::Submit, cx| this.send(cx))
			.detach();
		// Wake on backend events, or at a slow tick for notices and the login handoff.
		// Unchanged wakeups neither redraw nor allocate; a backlog drains in bounded batches.
		cx.spawn_in(window, async move |this, cx| {
			loop {
				let timer = cx.background_executor().timer(IDLE_TICK);
				let notified = std::pin::pin!(backend::WAKE.notified());
				futures_util::future::select(notified, timer).await;
				if this
					.update_in(cx, |this, window, cx| this.poll(window, cx))
					.is_err()
				{
					break;
				}
			}
		})
		.detach();
		let state = if demo {
			test_support::chat_demo_state()
		} else {
			State::default()
		};
		let rows = state.timeline.row_ids().collect::<Vec<_>>();
		let guild = state
			.selected
			.and_then(|id| state.channel(id))
			.and_then(|c| c.guild);
		let messages = ListState::new(rows.len(), ListAlignment::Bottom, px(600.));
		let view = cx.entity().downgrade();
		messages.set_scroll_handler(move |event, _, cx| {
			// The list is borrowed while this runs; request older pages after it returns.
			if event.visible_range.start < 3 && event.count > 0 {
				let view = view.clone();
				cx.defer(move |cx| {
					let _ = view.update(cx, |this, cx| this.load_older(cx));
				});
			}
		});
		let mut this = Self {
			messages,
			state,
			backend: backend::Backend::start(demo),
			guild,
			composer,
			rows,
			nav: Vec::new(),
			collapsed: BTreeSet::new(),
			members_open: true,
			member_rows: Vec::new(),
			member_key: None,
			format: ui::FormatCache::default(),
			hovered: None,
			revealed: BTreeSet::new(),
			boundary: None,
			notice: None,
			typists: 0,
			state_status: "",
			status: if demo {
				"Offline preview · synthetic data"
			} else {
				"Checking saved login…"
			},
			backend_status: "",
			authorized: false,
			#[cfg(not(target_os = "linux"))]
			login: None,
		};
		this.state_status = this.state.status;
		this.sync_channels();
		this.update_placeholder(cx);
		if demo {
			let command = this.state.request_members();
			this.dispatch(command);
		}
		this
	}

	/// Offline screenshot states: `--demo-channel=ID`, `--demo-dm`, `--demo-reply`,
	/// `--demo-hover`, `--demo-typing` and `--demo-sign-in`. Synthetic fixtures only.
	fn apply_demo_flags(&mut self, cx: &mut Context<Self>) {
		let args = std::env::args().collect::<Vec<_>>();
		let flag = |name: &str| args.iter().any(|arg| arg == name);
		if let Some(id) = args
			.iter()
			.find_map(|arg| arg.strip_prefix("--demo-channel="))
			.and_then(|id| id.parse().ok())
		{
			self.select(Id(id), cx);
		}
		if flag("--demo-dm") {
			self.select_section(None, cx);
		}
		if flag("--demo-reply")
			&& let Some(&last) = self.rows.last()
		{
			self.state.reply = Some(client_core::Reply::to(last));
		}
		if flag("--demo-typing")
			&& let Some(channel) = self.state.selected
		{
			let timestamp = std::time::SystemTime::now()
				.duration_since(std::time::UNIX_EPOCH)
				.map_or(0, |elapsed| elapsed.as_secs());
			self.state.apply(Envelope {
				generation: self.state.generation,
				event: Event::Typing(client_core::typing::Signal {
					channel,
					user: Id(2),
					timestamp,
				}),
			});
		}
		if flag("--demo-hover") {
			self.hovered = self.rows.iter().rev().nth(1).copied();
		}
		if flag("--demo-sign-in") {
			self.state = State::default();
			self.status = "No saved login. Choose Continue with Discord.";
		}
	}

	fn notify_user(&mut self, text: impl Into<SharedString>) {
		self.notice = Some((text.into(), Instant::now()));
	}

	fn poll(&mut self, window: &mut Window, cx: &mut Context<Self>) {
		let was_signed_in = self.state.user.is_some();
		let mut changed = false;
		let mut navigation_changed = false;
		#[cfg(not(target_os = "linux"))]
		if let Some(login) = &self.login {
			login.pump();
			if let Some(secret) = login.token() {
				self.login = None;
				self.state = State::default();
				self.backend = backend::Backend::with_secret(secret);
				self.backend_status = "";
				self.status = "Verifying Discord login…";
				changed = true;
			} else if login.expired() {
				self.login = None;
				self.status = "Login timed out; no session was accepted. Try again.";
				changed = true;
			}
		}
		let backend_status = *self.backend.status.borrow_and_update();
		if self.backend_status != backend_status {
			self.backend_status = backend_status;
			if !backend_status.is_empty() {
				self.status = backend_status;
				if self.state.user.is_some() && !self.state.demo && backend_status != "Connected" {
					self.notify_user(backend_status);
				}
			}
			changed = true;
		}
		let mut batch = Vec::new();
		for applied in 0..=EVENTS_PER_TICK {
			if applied == EVENTS_PER_TICK {
				backend::WAKE.notify_one();
				break;
			}
			let Ok(envelope) = self.backend.events.try_recv() else {
				break;
			};
			batch.push(envelope);
		}
		// Typing signals queued before these events apply first, so a message retires them.
		let mut typing = Vec::new();
		while typing.len() < 8
			&& let Ok(envelope) = self.backend.events.typing.try_recv()
		{
			typing.push(envelope);
		}
		for envelope in typing.into_iter().chain(batch) {
			navigation_changed |= matches!(
				&envelope.event,
				Event::Startup(_)
					| Event::Ready { .. }
					| Event::ChannelCreated(_)
					| Event::ChannelRestored(_)
					| Event::ChannelChanged(_)
					| Event::ThreadChanged { .. }
					| Event::ThreadRemoved { .. }
					| Event::ThreadsSync { .. }
					| Event::GuildChanged(_)
					| Event::GuildJoined(_)
					| Event::PermissionsChanged
					| Event::Permissions(_)
					| Event::Unavailable(_)
					| Event::Resync | Event::ChannelAction(_)
					| Event::ServerAction(_)
					| Event::GroupAction(_)
					| Event::UserAction(_)
					| Event::PostCreated { .. }
					| Event::Archives { .. }
					| Event::ForumPosts { .. }
			);
			self.state.apply(envelope);
			changed = true;
		}
		// Typists expire without an event; redraw when the visible set shrinks.
		let typists = self.state.typing_users(Instant::now()).count();
		if typists != self.typists {
			self.typists = typists;
			changed = true;
		}
		if self.state.status != self.state_status {
			self.state_status = self.state.status;
			if self.state.user.is_some() && !self.state_status.is_empty() {
				self.notify_user(self.state_status);
				changed = true;
			}
		}
		if self
			.notice
			.as_ref()
			.is_some_and(|(_, shown)| shown.elapsed() > NOTICE_TIME)
		{
			self.notice = None;
			changed = true;
		}
		if navigation_changed {
			self.sync_channels();
		}
		if changed {
			if !was_signed_in && self.state.user.is_some() && self.state.selected.is_none() {
				let first = self
					.state
					.channels
					.iter()
					.find(|c| text_channel(c) && self.state.can_view(c.id))
					.map(|c| c.id);
				if let Some(id) = first {
					self.select(id, cx);
				}
			}
			self.sync_rows();
			self.mark_read(window);
			cx.notify();
		}
	}

	/// Acknowledge the newest message only while it is on screen in the active window.
	fn mark_read(&mut self, window: &Window) {
		if !window.is_window_active() || self.messages.is_scrolled_to_end() != Some(true) {
			return;
		}
		if let Some(&last) = self.rows.last()
			&& self.state.search_target.is_none()
			&& !self.state.history_targeted
		{
			let command = self.state.prepare_mark_read(last);
			self.dispatch(command);
		}
	}

	fn sync_rows(&mut self) {
		let rows = self.state.timeline.row_ids().collect::<Vec<_>>();
		let prefix = self
			.rows
			.iter()
			.zip(&rows)
			.take_while(|(a, b)| a == b)
			.count();
		let suffix = self.rows[prefix..]
			.iter()
			.rev()
			.zip(rows[prefix..].iter().rev())
			.take_while(|(a, b)| a == b)
			.count();
		if rows == self.rows {
			self.messages.splice(0..rows.len(), rows.len());
		} else {
			// Dividers and grouping depend on the previous row: remeasure both neighbours.
			let start = prefix.saturating_sub(1);
			let suffix = suffix.saturating_sub(1);
			self.messages
				.splice(start..self.rows.len() - suffix, rows.len() - start - suffix);
			self.rows = rows;
		}
		if self.boundary.is_none() && !self.state.history_pending && !self.rows.is_empty() {
			self.boundary = Some(self.unread_boundary());
		}
		let members = self.members_key();
		if self.member_key != members {
			self.member_key = members;
			self.sync_members();
		}
	}

	/// The first message from someone else after the read marker, captured before it moves.
	fn unread_boundary(&self) -> Option<Id> {
		let channel = self.state.selected?;
		let read = self.state.read_marker(channel)??;
		let me = self.state.user.as_ref().map(|u| u.id);
		self.rows.iter().copied().find(|id| {
			*id > read
				&& self
					.state
					.timeline
					.get_display(*id)
					.is_some_and(|m| Some(m.author.id) != me)
		})
	}

	fn select_section(&mut self, guild: Option<Id>, cx: &mut Context<Self>) {
		let channel = self
			.state
			.channels
			.iter()
			.filter(|c| c.guild == guild && text_channel(c) && self.state.can_view(c.id))
			.min_by_key(|c| (c.parent_id.is_some(), c.position, c.id))
			.map(|c| c.id);
		if let Some(channel) = channel {
			self.select(channel, cx);
		} else if self.save_draft(cx) {
			self.guild = guild;
			self.sync_channels();
			self.state.selected = None;
			self.state.timeline.clear();
			self.state.history_pending = false;
			self.composer
				.update(cx, |input, cx| input.set_value(String::new(), cx));
			self.sync_rows();
			self.update_placeholder(cx);
			cx.notify();
		}
	}

	fn dispatch(&mut self, command: Option<Command>) {
		let Some(command) = command else {
			return;
		};
		if self.state.demo {
			let event = match command {
				Command::History {
					channel,
					request,
					before,
					..
				} => Event::History {
					channel,
					request,
					older: before.is_some(),
					messages: if before.is_some() {
						vec![]
					} else {
						(480..500)
							.map(|id| test_support::message(id, channel))
							.collect()
					},
				},
				Command::Send {
					channel,
					content,
					nonce,
					reply,
					..
				} => {
					let mut message =
						test_support::message(10000 + self.state.send_sequence, channel);
					message.content = content;
					message.author = self.state.user.clone().expect("demo user");
					message.nonce = Some(nonce.clone());
					message.reply_to = reply.map(client_core::Reply::target);
					message.reactions = Some(vec![]);
					Event::SendResult {
						nonce,
						result: Ok(message),
					}
				}
				Command::Members {
					guild,
					channel: Some(channel),
					request,
					..
				} => Event::Members(test_support::demo_members(guild, channel, request)),
				_ => {
					self.state.command_rejected(command);
					return;
				}
			};
			self.state.apply(Envelope {
				generation: self.state.generation,
				event,
			});
		} else if let Err(error) = self.backend.commands.try_send(command) {
			self.state.command_rejected(error.into_inner());
		}
	}

	fn save_draft(&mut self, cx: &mut Context<Self>) -> bool {
		if let Some(id) = self.state.selected {
			let value = self.composer.read(cx).value();
			let old = self.state.drafts.get(&id).map_or(0, String::capacity);
			if self.state.draft_bytes().saturating_sub(old) + value.len()
				> client_core::MAX_DRAFT_BYTES
			{
				self.notify_user(
					"Draft storage is full; keep or send this draft before switching.",
				);
				cx.notify();
				return false;
			}
			if value.is_empty() {
				self.state.drafts.remove(&id);
			} else {
				self.state.drafts.insert(id, value.to_owned());
			}
		}
		true
	}

	fn select(&mut self, id: Id, cx: &mut Context<Self>) {
		if !self.save_draft(cx) {
			return;
		}
		let changed = self.state.selected != Some(id);
		if changed {
			self.boundary = None;
		}
		let command = self.state.select(id);
		self.guild = self.state.channel(id).and_then(|c| c.guild);
		self.sync_channels();
		self.dispatch(command);
		if changed {
			self.state.reply = None;
			let members = self.state.request_members();
			self.dispatch(members);
		}
		let draft = self.state.drafts.get(&id).cloned().unwrap_or_default();
		self.composer
			.update(cx, |input, cx| input.set_value(draft, cx));
		self.update_placeholder(cx);
		self.sync_rows();
		if changed {
			self.messages.reset(self.rows.len());
			self.format.retain(|_| false);
			self.revealed.clear();
			self.hovered = None;
		}
		cx.notify();
	}

	fn update_placeholder(&self, cx: &mut Context<Self>) {
		let placeholder = match self.state.selected.and_then(|id| self.state.channel(id)) {
			Some(channel) if channel.guild.is_some() => format!("Message #{}", channel.name),
			Some(channel) => format!("Message @{}", channel_label(channel)),
			None => "Message".into(),
		};
		self.composer
			.update(cx, |input, cx| input.set_placeholder(placeholder, cx));
	}

	fn load_older(&mut self, cx: &mut Context<Self>) {
		if self.state.history_pending || self.state.older_exhausted {
			return;
		}
		let command = self.state.older_history();
		if command.is_some() {
			self.dispatch(command);
			self.sync_rows();
			cx.notify();
		}
	}

	fn send(&mut self, cx: &mut Context<Self>) {
		if !self.save_draft(cx) {
			return;
		}
		let command = self.state.prepare_send();
		if command.is_some() {
			self.dispatch(command);
			self.state.reply = None;
			self.composer
				.update(cx, |input, cx| input.set_value(String::new(), cx));
			self.sync_rows();
			self.messages.scroll_to_end();
		}
		cx.notify();
	}

	fn button(
		&self,
		id: &'static str,
		label: impl Into<SharedString>,
		primary: bool,
	) -> Stateful<Div> {
		let p = palette();
		div()
			.id(id)
			.focusable()
			.tab_stop(true)
			.focus(|d| d.border_color(color(p.text_strong)))
			.h(px(38.))
			.px_4()
			.flex()
			.items_center()
			.rounded(px(8.))
			.border_1()
			.border_color(gpui::transparent_black())
			.cursor_pointer()
			.font_weight(FontWeight::MEDIUM)
			.text_size(px(14.))
			.when(primary, |d| {
				d.bg(color(p.accent))
					.text_color(color(p.accent_text))
					.hover(|d| d.opacity(0.9))
			})
			.when(!primary, |d| {
				d.bg(color(p.raised))
					.text_color(color(p.text_strong))
					.hover(|d| d.bg(color(p.hover)))
			})
			.child(label.into())
	}

	pub(crate) fn icon_button(
		&self,
		id: impl Into<ElementId>,
		glyph: Icon,
		active: bool,
		label: &'static str,
	) -> Stateful<Div> {
		let p = palette();
		div()
			.id(id)
			.focusable()
			.tab_stop(true)
			.size(px(32.))
			.flex_none()
			.rounded(px(6.))
			.flex()
			.items_center()
			.justify_center()
			.cursor_pointer()
			.hover(|d| d.bg(color(p.hover)))
			.focus(|d| d.bg(color(p.hover)))
			.tooltip(tooltip(label))
			.child(icon(
				glyph,
				px(20.),
				color(if active { p.text_strong } else { p.muted }),
			))
	}

	fn title_bar(&self) -> impl IntoElement {
		let p = palette();
		let title = match self.guild {
			Some(id) => self
				.state
				.guild(id)
				.map_or_else(String::new, |g| g.name.clone()),
			None => "Direct Messages".into(),
		};
		div()
			.id("title-bar")
			.h(px(36.))
			.flex_none()
			.relative()
			.flex()
			.items_center()
			.justify_center()
			.on_mouse_down(MouseButton::Left, |event, window, _| {
				if event.click_count == 2 {
					window.titlebar_double_click();
				} else {
					window.start_window_move();
				}
			})
			.child(
				div()
					.text_size(px(14.))
					.font_weight(FontWeight::SEMIBOLD)
					.text_color(color(p.text_strong))
					.child(title),
			)
			.when(self.state.demo, |d| {
				d.child(
					div()
						.absolute()
						.right(px(10.))
						.top(px(5.))
						.h(px(26.))
						.px_3()
						.flex()
						.items_center()
						.rounded(px(8.))
						.border_1()
						.border_color(color(p.border))
						.text_size(px(11.))
						.font_weight(FontWeight::SEMIBOLD)
						.text_color(color(p.muted))
						.child("OFFLINE PREVIEW"),
				)
			})
			.when(!self.state.demo && !self.state.gateway_connected, |d| {
				d.child(
					div()
						.absolute()
						.right(px(10.))
						.top(px(5.))
						.h(px(26.))
						.px_3()
						.flex()
						.items_center()
						.rounded(px(8.))
						.bg(ui_warning_tint())
						.text_size(px(11.))
						.font_weight(FontWeight::SEMIBOLD)
						.text_color(color(p.warning))
						.child("RECONNECTING"),
				)
			})
	}

	fn notice_layer(&self, cx: &mut Context<Self>) -> Option<impl IntoElement> {
		let p = palette();
		let (text, _) = self.notice.as_ref()?;
		Some(
			div()
				.absolute()
				.bottom(px(84.))
				.right(px(20.))
				.max_w(px(360.))
				.child(
					div()
						.id("notice")
						.px_4()
						.py_3()
						.rounded(px(12.))
						.bg(color(p.base))
						.border_1()
						.border_color(color(p.border))
						.shadow_lg()
						.flex()
						.gap_3()
						.items_center()
						.cursor_pointer()
						.on_click(cx.listener(|this, _, _, cx| {
							this.notice = None;
							cx.notify();
						}))
						.child(
							div()
								.size(px(8.))
								.flex_none()
								.rounded_full()
								.bg(color(p.warning)),
						)
						.child(
							div()
								.text_sm()
								.text_color(color(p.text_strong))
								.child(text.clone()),
						),
				),
		)
	}
}

fn ui_warning_tint() -> Rgba {
	theme::tint(palette().warning, 0.14)
}

impl Render for Serein {
	fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
		let p = palette();
		let body = {
			#[cfg(not(target_os = "linux"))]
			if self.login.is_some() {
				Some(self.render_login(window, cx))
			} else {
				None
			}
			#[cfg(target_os = "linux")]
			None::<AnyElement>
		};
		let body = match body {
			Some(body) => body,
			None if self.state.user.is_none() => self.render_sign_in(cx),
			None => {
				let members = self.members_open && window.viewport_size().width >= px(900.);
				div()
					.flex_1()
					.min_h_0()
					.flex()
					.child(self.render_navigation(cx))
					.child(self.render_chat(cx))
					.when(members, |d| d.child(self.render_members(cx)))
					.into_any_element()
			}
		};
		let signed_in = self.state.user.is_some();
		#[cfg(not(target_os = "linux"))]
		let signed_in = signed_in && self.login.is_none();
		div()
			.on_key_down(tab_navigation)
			.size_full()
			.relative()
			.flex()
			.flex_col()
			.bg(color(p.base))
			.font_family(theme::FONT)
			.text_color(color(p.text))
			.text_size(px(15.))
			.when(signed_in, |d| d.child(self.title_bar()))
			.when(!signed_in && cfg!(target_os = "macos"), |d| {
				d.child(
					div()
						.h(px(28.))
						.flex_none()
						.on_mouse_down(MouseButton::Left, |_, window, _| {
							window.start_window_move()
						}),
				)
			})
			.child(body)
			.children(self.notice_layer(cx))
	}
}

fn main() {
	let demo = std::env::args().any(|arg| arg == "--demo");
	gpui_platform::application()
		.with_assets(theme::Assets)
		.run(move |cx: &mut App| {
			theme::install_fonts(cx);
			input::init(cx);
			cx.on_action(|_: &Quit, cx| cx.quit());
			cx.on_action(|_: &Hide, cx| cx.hide());
			cx.on_action(|_: &HideOthers, cx| cx.hide_other_apps());
			cx.on_action(|_: &ShowAll, cx| cx.unhide_other_apps());
			cx.on_action(|_: &Minimize, cx| {
				if let Some(window) = cx.active_window() {
					let _ = window.update(cx, |_, window, _| window.minimize_window());
				}
			});
			cx.bind_keys([
				KeyBinding::new("cmd-q", Quit, None),
				KeyBinding::new("cmd-h", Hide, None),
				KeyBinding::new("alt-cmd-h", HideOthers, None),
				KeyBinding::new("cmd-m", Minimize, None),
			]);
			// macOS routes Cut/Copy/Paste/Select All for the hosted login page through this menu;
			// without it WKWebView never receives those key equivalents.
			cx.set_menus([
				Menu::new("Serein").items([
					MenuItem::action("Hide Serein", Hide),
					MenuItem::action("Hide Others", HideOthers),
					MenuItem::action("Show All", ShowAll),
					MenuItem::separator(),
					MenuItem::action("Quit Serein", Quit),
				]),
				Menu::new("Edit").items([
					MenuItem::os_action("Cut", input::Cut, OsAction::Cut),
					MenuItem::os_action("Copy", input::Copy, OsAction::Copy),
					MenuItem::os_action("Paste", input::Paste, OsAction::Paste),
					MenuItem::os_action("Select All", input::SelectAll, OsAction::SelectAll),
				]),
				Menu::new("Window").items([MenuItem::action("Minimize", Minimize)]),
			]);
			cx.on_window_closed(|cx, _| {
				if cx.windows().is_empty() {
					cx.quit();
				}
			})
			.detach();
			let bounds = Bounds::centered(None, size(px(1180.), px(780.)), cx);
			let result = cx.open_window(
				WindowOptions {
					window_bounds: Some(WindowBounds::Windowed(bounds)),
					window_min_size: Some(size(px(760.), px(480.))),
					titlebar: Some(TitlebarOptions {
						title: Some("Serein".into()),
						appears_transparent: true,
						traffic_light_position: Some(point(px(12.), px(11.))),
					}),
					..Default::default()
				},
				|window, cx| {
					cx.new(|cx| {
						let mut serein = Serein::new(demo, window, cx);
						if demo {
							serein.apply_demo_flags(cx);
						}
						serein
					})
				},
			);
			if result.is_err() {
				eprintln!("Could not open the GPUI window.");
				cx.quit();
			}
			cx.activate(true);
		});
}
