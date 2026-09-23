//! Experimental native GPUI frontend; transports and secrets remain in Serein's shared crates.
mod autocomplete;
mod backend;
mod chat;
mod components;
mod emoji;
mod folders;
mod forum;
mod friends;
mod gif;
mod images;
mod input;
mod members;
mod nav_menu;
mod persist;
mod profile;
mod reactors;
mod search;
mod settings;
mod sidebar;
mod signin;
mod slash;
mod switcher;
mod theme;
mod threads;
mod uploads;

use client_core::{Command, Envelope, Event, State};
use gpui::{prelude::*, *};
use model::Id;
use std::{
	collections::BTreeSet,
	time::{Duration, Instant},
};
use theme::{Icon, color, icon, palette};

actions!(
	serein,
	[
		Quit,
		Hide,
		HideOthers,
		ShowAll,
		Minimize,
		ToggleSwitcher,
		OpenSettings
	]
);

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
	search_input: Entity<input::Input>,
	rows: Vec<Id>,
	nav: Vec<sidebar::NavRow>,
	collapsed: BTreeSet<Id>,
	members_open: bool,
	member_rows: Vec<members::MemberRow>,
	member_key: Option<members::MembersKey>,
	messages: ListState,
	/// Lowest timeline row laid out in the previous frame, for the unread banner.
	first_rendered: std::cell::Cell<usize>,
	format: ui::FormatCache,
	hovered: Option<Id>,
	profile: Option<profile::Card>,
	/// Anchor of the open "who reacted" popover.
	reactors_at: Option<Point<Pixels>>,
	picker: Option<autocomplete::Picker>,
	/// Slash-command picker, chosen command and its option fields.
	slash: slash::Slash,
	switcher: Option<switcher::Switcher>,
	/// Emoji popover for the composer or a message reaction.
	emoji_picker: Option<emoji::Picker>,
	gif_picker: Option<gif::Picker>,
	/// Name filter of the open Threads dialog.
	thread_filter: Option<Entity<input::Input>>,
	emoji_closed_at: Option<Instant>,
	/// Open string select: message and component `custom_id`.
	open_select: Option<(Id, String)>,
	/// Inline editor for one of your messages.
	editing: Option<(Id, Entity<input::Input>)>,
	/// Messages whose spoilers were revealed by a click; cleared on channel change.
	revealed: BTreeSet<Id>,
	/// First unread message when the channel opened; `None` until its history arrives.
	boundary: Option<Option<Id>>,
	/// Arriving in an unread conversation keeps it unread (and the banner up) until the reader
	/// scrolls down at the newest message or jumps to the present, as the main app does.
	hold_read_ack: bool,
	notice: Option<(SharedString, Instant)>,
	/// OS alerts for mentions and DMs while the window is inactive; off until opted in.
	alerts: platform::notifications::Notifications,
	/// Visible typists at the last redraw.
	typists: usize,
	/// Last `State::status` shown, so each new value becomes one transient notice.
	state_status: &'static str,
	status: &'static str,
	backend_status: &'static str,
	authorized: bool,
	/// User settings modal and the device choices it edits.
	settings: settings::Settings,
	/// Friends page on the home view, shown instead of the chat.
	friends: friends::Page,
	/// Post-list settings for forum and media channels.
	forum: forum::View,
	/// Rail/channel right-click menu and expanded server folders.
	navigation: sidebar::NavState,
	/// Composer attachments: chosen files and the upload in flight.
	uploads: uploads::Uploads,
	/// Settings, drafts and categories in the experiment's own local store.
	persist: persist::Persist,
	#[cfg(not(target_os = "linux"))]
	login: Option<platform::LoginView>,
}

impl Serein {
	fn new(demo: bool, window: &mut Window, cx: &mut Context<Self>) -> Self {
		let composer = cx.new(input::Input::new);
		let search_input = cx.new(input::Input::new);
		search_input.update(cx, |input, cx| {
			input.set_small(true);
			input.set_placeholder("Search".into(), cx)
		});
		cx.subscribe(&search_input, |this, _, _: &input::Submit, cx| {
			this.run_search(cx)
		})
		.detach();
		cx.subscribe(&search_input, |this, _, event: &input::Event, cx| {
			this.search_input_event(event, cx)
		})
		.detach();
		let settings = settings::Settings::new(window, cx);
		cx.subscribe(&composer, |this, _, _: &input::Submit, cx| this.send(cx))
			.detach();
		cx.subscribe_in(
			&composer,
			window,
			|this, _, event: &input::Event, window, cx| match event {
				input::Event::Cancel => {
					if !this.cancel_slash(cx) {
						this.state.reply = None;
					}
					cx.notify();
				}
				input::Event::Changed => this.update_picker(cx),
				input::Event::Pick(key) => this.pick(*key, window, cx),
				input::Event::EditLast => {
					let me = this.state.user.as_ref().map(|u| u.id);
					let last = this.rows.iter().rev().copied().find(|id| {
						this.state
							.timeline
							.get_display(*id)
							.is_some_and(|m| Some(m.author.id) == me)
					});
					if let Some(id) = last {
						this.start_edit(id, window, cx);
					}
				}
			},
		)
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
		// Queue the open draft and unsaved choices before the window or the app goes away.
		cx.on_app_quit(|this, cx| {
			this.queue_persist(true, cx);
			let done = this.persist.finish();
			cx.background_executor().spawn(async move {
				if let Some(done) = done {
					let _ = done.recv_timeout(persist::EXIT_WAIT);
				}
			})
		})
		.detach();
		let closing = cx.entity().downgrade();
		window.on_window_should_close(cx, move |_, cx| {
			let _ = closing.update(cx, |this, cx| this.queue_persist(true, cx));
			true
		});
		let state = if demo {
			// Synthetic avatars and server icons, drawn locally like the main app's preview.
			images::init_demo();
			demo_state(&std::env::args().collect::<Vec<_>>())
		} else {
			// Avatars and previews come from Discord's CDN; the offline preview never fetches.
			images::init();
			State::default()
		};
		let rows = state.timeline.row_ids().collect::<Vec<_>>();
		let guild = state
			.selected
			.and_then(|id| state.channel(id))
			.and_then(|c| c.guild);
		let messages = ListState::new(rows.len(), ListAlignment::Bottom, px(120.));
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
			first_rendered: std::cell::Cell::new(usize::MAX),
			state,
			backend: backend::Backend::start(demo),
			guild,
			composer,
			search_input,
			rows,
			nav: Vec::new(),
			collapsed: BTreeSet::new(),
			members_open: true,
			member_rows: Vec::new(),
			member_key: None,
			format: ui::FormatCache::default(),
			hovered: None,
			editing: None,
			profile: None,
			reactors_at: None,
			picker: None,
			slash: slash::Slash::default(),
			switcher: None,
			emoji_picker: None,
			gif_picker: None,
			thread_filter: None,
			emoji_closed_at: None,
			open_select: None,
			revealed: BTreeSet::new(),
			boundary: None,
			hold_read_ack: false,
			notice: None,
			alerts: platform::notifications::Notifications::new(|| backend::WAKE.notify_one()),
			typists: 0,
			state_status: "",
			status: if demo {
				"Offline preview · synthetic data"
			} else {
				"Checking saved login…"
			},
			backend_status: "",
			authorized: false,
			settings,
			friends: friends::Page::default(),
			forum: forum::View::default(),
			navigation: sidebar::NavState::default(),
			uploads: uploads::Uploads::default(),
			persist: persist::Persist::start(demo),
			#[cfg(not(target_os = "linux"))]
			login: None,
		};
		this.state_status = this.state.status;
		this.sync_channels();
		this.update_placeholder(cx);
		if demo {
			let command = this.state.request_members();
			this.dispatch(command);
			slash::demo_permissions(&mut this.state);
		}
		this
	}

	/// Offline screenshot states: `--demo-channel=ID`, `--demo-dm`, `--demo-reply`,
	/// `--demo-hover`, `--demo-own-hover`, `--demo-edit`, `--demo-profile`, `--demo-mention`, `--demo-emoji-picker`,
	/// `--demo-emoji-react`, `--demo-emoji-suggest`, `--demo-typing`, `--demo-forum` and `--demo-sign-in`. Synthetic fixtures only.
	fn apply_demo_flags(&mut self, window: &mut Window, cx: &mut Context<Self>) {
		self.settings.apply_demo_flags();
		// As the main app's preview: hidden channels listed, #getting-started a favourite and
		// Robin's DM pinned.
		self.settings.show_hidden_channels = true;
		self.navigation.favorites = vec![Id(20)];
		self.navigation.pinned = vec![Id(22)];
		self.sync_channels();
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
		if (flag("--demo-typing") || flag("--demo-reply"))
			&& let Some(channel) = self.state.selected
		{
			let timestamp = std::time::SystemTime::now()
				.duration_since(std::time::UNIX_EPOCH)
				.map_or(0, |elapsed| elapsed.as_secs());
			// The reply preview shows two typists, as the main app's.
			let users: &[u64] = if flag("--demo-reply") { &[2, 3] } else { &[2] };
			for user in users {
				self.state.apply(Envelope {
					generation: self.state.generation,
					event: Event::Typing(client_core::typing::Signal {
						channel,
						user: Id(*user),
						timestamp,
					}),
				});
			}
		}
		if flag("--demo-threads")
			&& let Some(channel) = self.state.selected
		{
			self.open_threads(channel, cx);
		}
		if let Some(section) = args.iter().find_map(|arg| {
			arg.strip_prefix("--demo-gifs")
				.map(|rest| rest.strip_prefix('=').unwrap_or("").to_owned())
		}) {
			self.preview_gif_picker(&section, window, cx);
		}
		if flag("--demo-hover") {
			self.hovered = self.rows.iter().rev().nth(1).copied();
		}
		if flag("--demo-edit") {
			let me = self.state.user.as_ref().map(|u| u.id);
			if let Some(id) = self.rows.iter().rev().copied().find(|id| {
				self.state
					.timeline
					.get_display(*id)
					.is_some_and(|m| Some(m.author.id) == me)
			}) {
				self.start_edit(id, window, cx);
			}
		}
		if flag("--demo-own-hover") {
			let me = self.state.user.as_ref().map(|u| u.id);
			self.hovered = self.rows.iter().rev().copied().find(|id| {
				self.state
					.timeline
					.get_display(*id)
					.is_some_and(|m| Some(m.author.id) == me)
			});
		}
		// The same synthetic member and profile card as the main app's `--demo-profile`,
		// opened beside the member list.
		if flag("--demo-profile") {
			let user = test_support::message(1, Id(20)).author;
			let guild = self.state.channel(Id(20)).and_then(|c| c.guild);
			let anchor = point(window.viewport_size().width - px(members::WIDTH), px(48.));
			self.open_profile(user, guild, vec![], anchor, cx);
		}
		if flag("--demo-emoji-suggest") {
			self.composer
				.update(cx, |input, cx| input.set_value("Nice :th".into(), cx));
			self.update_picker(cx);
		}
		let viewport = window.viewport_size();
		if flag("--demo-emoji-picker") {
			let position = point(viewport.width - px(64.), viewport.height - px(64.));
			self.open_emoji_picker(emoji::Target::Composer, position, window, cx);
		}
		if flag("--demo-emoji-react")
			&& let Some(&id) = self.rows.iter().rev().nth(1)
		{
			self.hovered = Some(id);
			let position = point(viewport.width - px(280.), viewport.height - px(320.));
			self.open_emoji_picker(emoji::Target::React(id), position, window, cx);
		}
		if flag("--demo-mention") {
			self.composer
				.update(cx, |input, cx| input.set_value("Thanks @".into(), cx));
			self.update_picker(cx);
		}
		if let Some(query) = args
			.iter()
			.find_map(|arg| arg.strip_prefix("--demo-search="))
		{
			let query = query.to_owned();
			self.search_input
				.update(cx, |input, cx| input.set_value(query, cx));
			self.run_search(cx);
		}
		if flag("--demo-pins") {
			self.toggle_pins(cx);
		}
		if flag("--demo-components")
			&& let Some(channel) = self.state.selected
		{
			let button = |id: u32, style: u8, label: &str| model::Component {
				kind: 2,
				id,
				style: Some(style),
				label: Some(label.into()),
				custom_id: (style != 5).then(|| format!("button-{id}")),
				url: (style == 5).then(|| "https://example.com".into()),
				..Default::default()
			};
			let mut message = test_support::message(9000, channel);
			message.author.name = "Synthetic support".into();
			message.author.kind = model::AccountKind::Bot;
			message.content = "Component preview — all interactions stay offline.".into();
			message.components = vec![
				model::Component {
					kind: 17,
					id: 1,
					accent_color: Some(0x1a72e8),
					components: vec![
						model::Component {
							kind: 10,
							id: 2,
							content: Some("## Support\nChoose a **topic** below.".into()),
							..Default::default()
						},
						model::Component {
							kind: 14,
							id: 3,
							divider: Some(true),
							..Default::default()
						},
						model::Component {
							kind: 1,
							id: 4,
							components: vec![model::Component {
								kind: 3,
								id: 5,
								custom_id: Some("topic".into()),
								placeholder: Some("Choose a support topic".into()),
								options: vec![
									model::ComponentOption {
										label: "Support".into(),
										value: "support".into(),
										description: Some("General questions".into()),
										..Default::default()
									},
									model::ComponentOption {
										label: "Billing".into(),
										value: "billing".into(),
										description: Some("Purchases and invoices".into()),
										..Default::default()
									},
								],
								..Default::default()
							}],
							..Default::default()
						},
					],
					..Default::default()
				},
				model::Component {
					kind: 1,
					id: 6,
					components: vec![
						button(7, 1, "Open form"),
						button(8, 2, "Later"),
						button(9, 4, "Close ticket"),
						button(10, 5, "Documentation"),
					],
					..Default::default()
				},
			];
			let _ = self.state.timeline.insert(message, false, false);
			self.open_select = Some((Id(9000), "topic".into()));
			self.sync_rows();
		}
		// Navigation: `--demo-rail` (mentions and unread DMs, as `notification_demo_state`),
		// `--demo-folders`/`--demo-folder-open`, `--demo-friends[-all|-pending]`,
		// `--demo-channel-menu` and `--demo-server-menu`.
		if flag("--demo-rail")
			&& let Some(me) = self.state.user.clone()
		{
			for (id, channel) in [(1001, Id(22)), (1003, Id(22)), (1005, Id(21))] {
				let mut message = test_support::message(id, channel);
				message.mentions = vec![me.clone()];
				self.state.apply(Envelope {
					generation: self.state.generation,
					event: Event::Message(message),
				});
			}
		}
		if flag("--demo-folders") || flag("--demo-folder-open") {
			test_support::seed_demo_folder_mosaic(&mut self.state);
			if flag("--demo-folder-open") {
				self.navigation.expanded.insert(1);
			}
		}
		for (name, tab) in [
			("--demo-friends", friends::Tab::Online),
			("--demo-friends-all", friends::Tab::All),
			("--demo-friends-pending", friends::Tab::Pending),
		] {
			if flag(name) {
				// The same synthetic friend presences as `test_support::friends_demo_state`.
				self.state.apply(Envelope {
					generation: self.state.generation,
					event: Event::DirectPresence(
						(0..16)
							.map(|i| client_core::presence::Update {
								user: Id(1001 + i),
								status: model::Patch::Value(
									if i < 7 { "online" } else { "offline" }.into(),
								),
								custom_status: model::Patch::Null,
								activities: model::Patch::Value(Vec::new()),
							})
							.collect(),
					),
				});
				self.friends.tab = tab;
				self.open_friends(cx);
			}
		}
		if flag("--demo-channel-menu") {
			let target = nav_menu::Target::Channel(self.state.selected.unwrap_or(Id(21)));
			self.open_nav_menu(target, point(px(230.), px(150.)), window, cx);
		}
		if flag("--demo-server-menu") {
			let target = nav_menu::Target::Guild(Id(10));
			self.open_nav_menu(target, point(px(44.), px(130.)), window, cx);
		}
		// The fixture already mutes #long-form; `--demo-hide-muted` hides it from the list,
		// `--demo-mute-menu`/`--demo-notification-menu` open the channel menu's submenus,
		// `--demo-category-menu`, `--demo-dm-menu`, `--demo-group-menu` and
		// `--demo-friend-menu` the other menus, `--demo-add-friend` the Add Friend tab.
		// `--demo-hidden-channels` adds the access fixture and turns on "Show hidden channels".
		if flag("--demo-hidden-channels") {
			test_support::seed_access_marks(&mut self.state);
			self.settings.show_hidden_channels = true;
			self.sync_channels();
		}
		if flag("--demo-hide-muted") {
			let command = self.state.request_channel_action(
				Id(20),
				client_core::channel_actions::Action::HideMuted(true),
			);
			self.dispatch(command);
			self.sync_channels();
		}
		for (name, page) in [
			("--demo-mute-menu", nav_menu::Page::Mute),
			("--demo-notification-menu", nav_menu::Page::Notifications),
		] {
			if flag(name) {
				let target = nav_menu::Target::Channel(self.state.selected.unwrap_or(Id(21)));
				self.open_nav_menu(target, point(px(230.), px(150.)), window, cx);
				self.set_nav_menu_page(page);
			}
		}
		if flag("--demo-category-menu") {
			self.open_nav_menu(
				nav_menu::Target::Channel(Id(24)),
				point(px(230.), px(150.)),
				window,
				cx,
			);
		}
		for (name, channel) in [("--demo-dm-menu", Id(22)), ("--demo-group-menu", Id(29))] {
			if flag(name) {
				self.select_section(None, cx);
				let target = nav_menu::Target::Channel(channel);
				self.open_nav_menu(target, point(px(230.), px(150.)), window, cx);
			}
		}
		if flag("--demo-friend-menu")
			&& let Some(&(user, _)) = friends::rows(&self.state, friends::Tab::All).first()
		{
			self.friends.tab = friends::Tab::All;
			self.open_friends(cx);
			self.open_nav_menu(
				nav_menu::Target::Friend(user),
				point(px(640.), px(150.)),
				window,
				cx,
			);
		}
		if flag("--demo-add-friend") {
			self.friends.tab = friends::Tab::AddFriend;
			self.open_friends(cx);
			self.focus_friend_username(window, cx);
		}
		if let Some(query) = args
			.iter()
			.find_map(|arg| arg.strip_prefix("--demo-switcher="))
		{
			let query = query.to_owned();
			self.toggle_switcher(window, cx);
			if let Some(switcher) = &self.switcher {
				let input = switcher.input.clone();
				input.update(cx, |input, cx| input.set_value(query, cx));
			}
			self.refresh_switcher(cx);
		}
		// `--demo-forum` opens the synthetic "ideas" forum with seeded post summaries.
		if flag("--demo-forum")
			&& let Some(forum) = self
				.state
				.channels
				.iter()
				.find(|c| self.state.is_forum(c.id))
				.map(|c| c.id)
		{
			self.select(forum, cx);
		}
		// `--demo-attachments` shows two synthetic chosen files (nothing is read from disk);
		// `--demo-upload-progress` freezes a synthetic upload part-way.
		if let Some(channel) = self.state.selected {
			let generation = self.state.generation;
			if flag("--demo-attachments") {
				let files = [
					("launch-notes.pdf", 248_832),
					("harbour-sunset.png", 1_843_200),
				];
				self.uploads.demo_select(generation, channel, &files);
			}
			if flag("--demo-upload-progress") {
				self.uploads
					.demo_progress(generation, channel, 2, 1_310_720, 2_092_032);
			}
		}
		if flag("--demo-reactors")
			&& let Some(&last) = self.rows.last()
		{
			let emoji = model::ReactionEmoji {
				id: None,
				name: Some("👍".into()),
			};
			let _ = self.state.timeline.set_reactions(
				last,
				Some(vec![model::Reaction {
					emoji: emoji.clone(),
					count: 3,
					me: false,
					me_burst: false,
				}]),
			);
			self.sync_rows();
			self.open_reactors(last, emoji, point(px(700.), px(420.)), cx);
		}
		if flag("--demo-video")
			&& let Some(&last) = self.rows.last()
			&& let Some(mut message) = self.state.timeline.get(last).cloned()
		{
			message.attachments.push(model::Attachment {
				id: Id(9100),
				filename: "harbour-timelapse.mp4".into(),
				description: None,
				content_type: Some("video/mp4".into()),
				size: 8_400_000,
				media: model::EmbedMedia {
					width: 1920,
					height: 1080,
					..Default::default()
				},
				spoiler: false,
				duration_ms: None,
				waveform: vec![],
			});
			let _ = self.state.timeline.insert(message, true, false);
			self.sync_rows();
		}
		// `--demo-slash` opens the command picker; `--demo-slash-options` chooses /weather with
		// filled, one out-of-range, option; `--demo-slash-reply` runs it for a private reply.
		if flag("--demo-slash") || flag("--demo-slash-options") || flag("--demo-slash-reply") {
			self.composer
				.update(cx, |input, cx| input.set_value("/".into(), cx));
			self.update_picker(cx);
			if !flag("--demo-slash") {
				self.demo_slash_options(flag("--demo-slash-reply"), window, cx);
			}
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
		// Always drain, so the reducer's bounded queue never holds stale alerts.
		let active = window.is_window_active();
		while let Some(notification) = self.state.take_notification() {
			if self.state.demo || (active && self.state.selected == Some(notification.channel)) {
				continue;
			}
			let title: String = notification.sender.chars().take(64).collect();
			let body: String = notification.preview.chars().take(180).collect();
			let _ = self
				.alerts
				.notify_channel(notification.channel, title, body, None);
		}
		if let Some(channel) = self.alerts.take_activation() {
			window.activate_window();
			if self.state.channel(channel).is_some() {
				self.select(channel, cx);
			}
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
		changed |= images::drain(window, cx);
		changed |= self.poll_slash(cx);
		let attach = self
			.state
			.selected
			.is_some_and(|channel| self.state.demo || self.state.can_attach(channel));
		changed |= self
			.uploads
			.poll(self.state.generation, self.state.selected, attach);
		if let Some(problem) = self.uploads.take_notice() {
			self.notify_user(problem);
			changed = true;
		}
		changed |= self.poll_persist(window, cx);
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
		if self.hold_read_ack
			&& self.state.selected.and_then(|c| self.state.missed(c)) != Some(true)
		{
			self.hold_read_ack = false;
		}
		if self.hold_read_ack
			|| !window.is_window_active()
			|| self.messages.is_scrolled_to_end() != Some(true)
		{
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
		splice_rows(&self.messages, &mut self.rows, rows);
		if self.boundary.is_none() && !self.state.history_pending && !self.rows.is_empty() {
			self.boundary = Some(self.unread_boundary());
			self.hold_read_ack = self
				.state
				.selected
				.is_some_and(|c| self.state.unread(c) == Some(true));
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
			if let Some(event) = self.demo_search(&command) {
				self.state.apply(Envelope {
					generation: self.state.generation,
					event,
				});
				return;
			}
			let event = match command {
				Command::CancelSearch => return,
				command @ (Command::ApplicationCommands { .. } | Command::Interaction(_)) => {
					slash::demo_respond(&mut self.state, command);
					return;
				}
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
					// Synthetic metadata for files "uploaded" offline; no bytes leave the app.
					let files = self.uploads.take_demo_sent();
					if !files.is_empty() {
						message.attachments = files;
					}
					Event::SendResult {
						nonce,
						result: Ok(message),
					}
				}
				Command::Edit {
					request,
					channel,
					message,
					content,
				} => {
					let result = self
						.state
						.timeline
						.get(message)
						.cloned()
						.ok_or(client_core::auth::Failure::Protocol)
						.map(|mut updated| {
							updated.content = content;
							updated.edited = true;
							updated.edited_at =
								Some(updated.edited_at.unwrap_or(0).saturating_add(1));
							updated
						});
					Event::Edited {
						request,
						channel,
						message,
						result,
					}
				}
				Command::Reactions(command) => {
					use client_core::reactions::{Command as R, Event as E};
					Event::Reactions(match command {
						R::Read {
							channel,
							message,
							request,
						} => E::Read {
							channel,
							message,
							request,
							result: Ok(vec![]),
						},
						// Same synthetic-RAM toggle as the desktop fixture; nothing is sent.
						R::Set {
							channel,
							message,
							emoji,
							add,
							request,
						} => {
							let mut reactions = self
								.state
								.timeline
								.get(message)
								.and_then(|m| m.reactions.clone())
								.unwrap_or_default();
							if let Some(r) = reactions.iter_mut().find(|r| r.emoji.same(&emoji)) {
								if r.me != add {
									r.count = if add {
										r.count + 1
									} else {
										r.count.saturating_sub(1)
									};
									r.me = add;
								}
							} else if add {
								reactions.push(model::Reaction {
									emoji,
									count: 1,
									me: true,
									me_burst: false,
								});
							}
							reactions.retain(|r| r.count > 0);
							self.state.reactions.reset();
							let _ = self.state.timeline.set_reactions(message, Some(reactions));
							E::Written {
								channel,
								message,
								request,
								result: Ok(()),
							}
						}
						R::Users {
							channel,
							message,
							emoji,
							request,
							..
						} => E::Users {
							channel,
							message,
							emoji,
							request,
							result: Ok((1..=3)
								.map(|id| test_support::message(id, channel).author)
								.collect()),
						},
					})
				}
				Command::Delete { channel, message } => Event::Delete {
					channel,
					id: message,
				},
				Command::Pin {
					request,
					channel,
					message,
					pinned,
				} => Event::Pinned {
					request,
					channel,
					message,
					pinned,
					result: Ok(()),
				},
				Command::EditProfile {
					user,
					request,
					changes,
				} => settings::demo_profile_edit(&self.state, user, request, changes),
				Command::Members {
					guild,
					channel: Some(channel),
					request,
					..
				} => Event::Members(test_support::demo_members(guild, channel, request)),
				Command::MarkRead {
					channel,
					message,
					request,
					..
				} => Event::ReadState(client_core::read_state::Event::Result {
					channel,
					message,
					request,
					result: Ok(()),
				}),
				Command::MarkGuildRead { guild, request } => {
					Event::ReadState(client_core::read_state::Event::GuildAck {
						guild,
						request,
						result: Ok(()),
					})
				}
				Command::UserAction {
					action, request, ..
				} => {
					for event in friends::demo_user_events(&self.state, action, request) {
						self.state.apply(Envelope {
							generation: self.state.generation,
							event,
						});
					}
					return;
				}
				// Personal mute/notification/hide-muted writes, as the desktop `channel_demo`.
				Command::ChannelAction {
					guild,
					channel,
					request,
					action,
				} if nav_menu::demo_channel_outcome(&action).is_some() => {
					Event::ChannelAction(client_core::channel_actions::Event::Finished {
						guild,
						channel,
						request,
						result: Ok(nav_menu::demo_channel_outcome(&action).expect("checked")),
					})
				}
				Command::ServerAction {
					action: action @ client_core::server_actions::Action::Leave(_),
					request,
				} => Event::ServerAction(client_core::server_actions::Event::Written {
					action,
					request,
					result: Ok(None),
				}),
				Command::GroupAction {
					action: client_core::group_actions::Action::Leave(channel),
					request,
				} => Event::GroupAction(client_core::group_actions::Event::Written {
					channel,
					request,
					result: Ok(None),
				}),
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
		self.queue_persist(true, cx);
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
		if self.state.selected == Some(id) && self.state.is_forum(id) {
			self.open_forum(id);
		}
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
			self.editing = None;
			self.profile = None;
			self.reactors_at = None;
			self.picker = None;
			self.emoji_picker = None;
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

	pub(crate) fn start_edit(&mut self, id: Id, window: &mut Window, cx: &mut Context<Self>) {
		let Some(channel) = self.state.selected else {
			return;
		};
		if !self.state.can_edit(channel, id) {
			return;
		}
		let Some(content) = self.state.timeline.get(id).map(|m| m.content.clone()) else {
			return;
		};
		let editor = cx.new(input::Input::new);
		editor.update(cx, |input, cx| {
			input.set_placeholder("Edit message".into(), cx);
			input.set_value(content, cx);
		});
		cx.subscribe(&editor, |this, _, _: &input::Submit, cx| this.save_edit(cx))
			.detach();
		cx.subscribe_in(
			&editor,
			window,
			|this, _, event: &input::Event, window, cx| {
				if matches!(event, input::Event::Cancel) {
					this.cancel_edit(window, cx);
				}
			},
		)
		.detach();
		let focus = editor.read(cx).focus_handle(cx);
		window.focus(&focus, cx);
		self.editing = Some((id, editor));
		self.messages.remeasure();
		cx.notify();
	}

	pub(crate) fn cancel_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
		self.editing = None;
		self.messages.remeasure();
		let focus = self.composer.read(cx).focus_handle(cx);
		window.focus(&focus, cx);
		cx.notify();
	}

	pub(crate) fn save_edit(&mut self, cx: &mut Context<Self>) {
		let (Some(channel), Some((id, editor))) = (self.state.selected, self.editing.take()) else {
			return;
		};
		let content = editor.read(cx).value().to_owned();
		let unchanged = self
			.state
			.timeline
			.get(id)
			.is_some_and(|m| m.content == content);
		if !unchanged {
			if content.trim().is_empty() {
				self.notify_user("Delete the message instead of saving it empty.");
				self.editing = Some((id, editor));
				cx.notify();
				return;
			}
			let command = self.state.prepare_edit(channel, id, content);
			self.dispatch(command);
		}
		self.messages.remeasure();
		cx.notify();
	}

	/// Removes the shared saved login after confirmation; the main app uses the same entry.
	fn confirm_log_out(&mut self, window: &mut Window, cx: &mut Context<Self>) {
		let account = self.state.user.as_ref().map(|user| user.id);
		let answer = window.prompt(
			PromptLevel::Warning,
			"Log out of Serein?",
			Some(
				"This removes the saved login from the OS credential store, which also signs out the main Serein app on this device.",
			),
			&["Log out", "Cancel"],
			cx,
		);
		cx.spawn(async move |this, cx| {
			if answer.await != Ok(0) {
				return;
			}
			let forgotten = cx
				.background_executor()
				.spawn(async move {
					platform::forget_session().is_ok()
						&& account.is_none_or(|id| platform::forget_account_session(id).is_ok())
				})
				.await;
			let _ = this.update(cx, |this, cx| {
				this.backend = backend::Backend::idle();
				this.backend_status = "";
				this.alerts.clear();
				if let Some(account) = account {
					this.persist.forget(account);
				}
				this.state = State::default();
				this.rows.clear();
				this.messages.reset(0);
				this.nav.clear();
				this.authorized = false;
				this.status = if forgotten {
					"Signed out. The saved login was removed."
				} else {
					"Signed out, but the OS credential store could not remove the saved login."
				};
				cx.notify();
			});
		})
		.detach();
	}

	/// Deletion is irreversible, so it always asks first.
	pub(crate) fn confirm_delete(&mut self, id: Id, window: &mut Window, cx: &mut Context<Self>) {
		let Some(channel) = self.state.selected else {
			return;
		};
		let answer = window.prompt(
			PromptLevel::Warning,
			"Delete this message?",
			Some("This cannot be undone."),
			&["Delete", "Cancel"],
			cx,
		);
		cx.spawn(async move |this, cx| {
			if answer.await == Ok(0) {
				let _ = this.update(cx, |this, cx| {
					let command = this.state.prepare_delete(channel, id);
					this.dispatch(command);
					this.sync_rows();
					cx.notify();
				});
			}
		})
		.detach();
	}

	pub(crate) fn toggle_pin(&mut self, id: Id, cx: &mut Context<Self>) {
		let Some(channel) = self.state.selected else {
			return;
		};
		let pinned = !self.state.is_pinned(channel, id);
		let command = self.state.prepare_pin(channel, id, pinned);
		if command.is_some() {
			self.notify_user(if pinned {
				"Message pinned"
			} else {
				"Message unpinned"
			});
		}
		self.dispatch(command);
		cx.notify();
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
		if self.run_slash(cx) {
			return;
		}
		if !self.save_draft(cx) {
			return;
		}
		let sent = if self.uploads.has_files() {
			self.send_with_attachments()
		} else {
			let command = self.state.prepare_send();
			let sent = command.is_some();
			self.dispatch(command);
			sent
		};
		if sent {
			self.state.reply = None;
			// Sending follows the newest message, as the main app does.
			self.hold_read_ack = false;
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

/// The offline fixture, chosen like the main app's `--demo` so screens can be compared side by
/// side: `--demo` alone is the full synthetic workspace, `--demo-chat`, `--demo-code`, ... pick
/// the same fixtures as egui. GPUI-only screenshot flags are written against the chat fixture,
/// so any other `--demo-*` flag (besides appearance and settings) selects it too.
fn demo_state(args: &[String]) -> State {
	let flag = |name: &str| args.iter().any(|arg| arg == name);
	let mut state = if flag("--demo-forwarded") {
		test_support::forwarded_demo_state()
	} else if flag("--demo-audio") || flag("--demo-voice-messages") {
		test_support::audio_demo_state()
	} else if flag("--demo-video-playing") || flag("--demo-video-paused") {
		test_support::video_demo_state()
	} else if flag("--demo-system-messages") {
		test_support::system_demo_state()
	} else if flag("--demo-code") {
		test_support::code_demo_state()
	} else if flag("--demo-notifications") {
		test_support::notification_demo_state()
	} else if [
		"--demo-friends",
		"--demo-friends-all",
		"--demo-friends-pending",
	]
	.iter()
	.any(|name| flag(name))
	{
		test_support::friends_demo_state()
	} else if flag("--demo-empty-channel") || flag("--demo-empty-channel-long") {
		test_support::empty_channel_demo_state(flag("--demo-empty-channel-long"))
	} else if args.iter().skip(1).any(|arg| {
		arg.starts_with("--demo-")
			// The reply preview uses the full workspace fixture, as in the main app.
			&& !["--demo-light", "--demo-dark", "--demo-reply", "--demo-threads"]
				.contains(&arg.as_str())
			&& !arg.starts_with("--demo-gifs")
			&& !arg.starts_with("--demo-theme=")
			&& !arg.starts_with("--demo-settings")
			// The main app's profile preview uses its full workspace fixture.
			&& arg != "--demo-profile"
	}) {
		test_support::chat_demo_state()
	} else {
		let mut state = test_support::demo_state();
		test_support::seed_demo_folder_mosaic(&mut state);
		test_support::seed_access_marks(&mut state);
		state
	};
	// The main app's synthetic roles, so author names and members wear the same colours.
	for guild in state.permissions.guilds.values_mut() {
		if let Some(roles) = &mut guild.roles {
			for (id, name, color, position) in [
				(9001, "Founders", 0xe78284, 2),
				(9002, "Community", 0xe5c769, 1),
			] {
				roles.push(model::permissions::Role {
					id: Id(id),
					bits: 0,
					name: name.into(),
					color,
					position,
					hoist: true,
				});
			}
		}
	}
	state
}

/// Updates `list` from `current` to `rows`, remeasuring the changed span and both neighbours
/// (dividers and grouping depend on the previous row).
///
/// `ListState::splice` moves the scroll top to the start of a replaced range that contains it,
/// so a page of older history would leave the reader at the new first row, which requests the
/// next page, forever. The message at the top of the view is kept in place instead; a list
/// following the newest message keeps following.
fn splice_rows(list: &ListState, current: &mut Vec<Id>, rows: Vec<Id>) {
	if rows == *current {
		// Content (edits, reactions, embeds) may have changed height; this keeps the position.
		list.remeasure_items(0..rows.len());
		return;
	}
	let prefix = current
		.iter()
		.zip(&rows)
		.take_while(|(a, b)| a == b)
		.count();
	let suffix = current[prefix..]
		.iter()
		.rev()
		.zip(rows[prefix..].iter().rev())
		.take_while(|(a, b)| a == b)
		.count();
	let top = list.logical_scroll_top();
	let anchor = current.get(top.item_ix).map(|id| (*id, top.offset_in_item));
	let start = prefix.saturating_sub(1);
	let suffix = suffix.saturating_sub(1);
	list.splice(start..current.len() - suffix, rows.len() - start - suffix);
	*current = rows;
	if let Some((id, offset_in_item)) = anchor
		&& let Some(item_ix) = current.iter().position(|row| *row == id)
	{
		list.scroll_to(ListOffset {
			item_ix,
			offset_in_item,
		});
	}
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
					.child(self.render_navigation(window, cx))
					.map(|d| {
						if self.friends_visible() {
							d.child(self.render_friends(cx))
						} else if self.forum_visible() {
							d.child(self.render_forum(cx))
						} else {
							d.child(self.render_chat(cx))
						}
					})
					.when(self.search_open(), |d| d.child(self.render_search(cx)))
					.when(
						members && !self.search_open() && !self.friends_visible(),
						|d| d.child(self.render_members(cx)),
					)
					.into_any_element()
			}
		};
		let signed_in = self.state.user.is_some();
		#[cfg(not(target_os = "linux"))]
		let signed_in = signed_in && self.login.is_none();
		div()
			.on_key_down(tab_navigation)
			.on_action(
				cx.listener(|this, _: &ToggleSwitcher, window, cx| {
					this.toggle_switcher(window, cx)
				}),
			)
			.on_action(cx.listener(|this, _: &OpenSettings, window, cx| {
				if this.state.user.is_some() {
					this.open_settings(None, window, cx);
				}
			}))
			.size_full()
			.relative()
			.flex()
			.flex_col()
			.bg(theme::window_background())
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
			.children(self.render_profile(window, cx))
			.children(self.render_reactors(cx))
			.children(self.render_switcher(cx))
			.children(self.render_emoji_picker(cx))
			.children(self.render_threads(window, cx))
			.children(self.render_settings(window, cx))
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
				KeyBinding::new("cmd-k", ToggleSwitcher, None),
				KeyBinding::new("ctrl-k", ToggleSwitcher, None),
				KeyBinding::new("cmd-,", OpenSettings, None),
				KeyBinding::new("ctrl-,", OpenSettings, None),
			]);
			// macOS routes Cut/Copy/Paste/Select All for the hosted login page through this menu;
			// without it WKWebView never receives those key equivalents.
			cx.set_menus([
				Menu::new("Serein").items([
					MenuItem::action("Settings…", OpenSettings),
					MenuItem::separator(),
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
			// "Remember window size": the offline preview never reads the file.
			let memory = (!demo)
				.then(dirs::data_local_dir)
				.flatten()
				.map(|dir| persist::load_window(&persist::window_path(&dir)));
			let displays = cx
				.primary_display()
				.into_iter()
				.chain(cx.displays())
				.collect::<Vec<_>>();
			let areas = displays
				.iter()
				.map(|d| (d.uuid().ok().map(|u| u.to_string()), d.visible_bounds()))
				.collect::<Vec<_>>();
			let placement = memory
				.as_ref()
				.filter(|memory| memory.remember)
				.and_then(|memory| memory.placement.as_ref());
			let (display_id, window_bounds) = match placement
				.and_then(|saved| Some((saved, persist::place_window(saved, &areas)?)))
			{
				Some((saved, (index, bounds))) => (
					Some(displays[index].id()),
					if saved.maximized {
						WindowBounds::Maximized(bounds)
					} else {
						WindowBounds::Windowed(bounds)
					},
				),
				None => (
					None,
					WindowBounds::Windowed(Bounds::centered(None, size(px(1180.), px(780.)), cx)),
				),
			};
			let (min_width, min_height) = persist::MIN_WINDOW;
			let result = cx.open_window(
				WindowOptions {
					window_bounds: Some(window_bounds),
					display_id,
					window_min_size: Some(size(px(min_width), px(min_height))),
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
							serein.apply_demo_flags(window, cx);
						}
						if let Some(memory) = memory {
							serein.restore_window_memory(memory);
						}
						cx.observe_window_bounds(window, |this, window, cx| {
							this.window_bounds_changed(window, cx)
						})
						.detach();
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

#[cfg(test)]
mod tests {
	use super::splice_rows;
	use gpui::{ListAlignment, ListOffset, ListState, px};
	use model::Id;

	fn ids(values: &[u64]) -> Vec<Id> {
		values.iter().copied().map(Id).collect()
	}

	#[test]
	fn older_history_keeps_the_message_being_read_in_view() {
		let list = ListState::new(3, ListAlignment::Bottom, px(120.));
		let mut rows = ids(&[30, 40, 50]);
		list.scroll_to(ListOffset {
			item_ix: 0,
			offset_in_item: px(5.),
		});
		splice_rows(&list, &mut rows, ids(&[10, 20, 30, 40, 50]));
		let top = list.logical_scroll_top();
		assert_eq!((top.item_ix, top.offset_in_item), (2, px(5.)));
		assert_eq!(list.item_count(), 5);
	}

	#[test]
	fn unchanged_rows_keep_the_scroll_position() {
		let list = ListState::new(3, ListAlignment::Bottom, px(120.));
		let mut rows = ids(&[30, 40, 50]);
		list.scroll_to(ListOffset {
			item_ix: 1,
			offset_in_item: px(3.),
		});
		splice_rows(&list, &mut rows, ids(&[30, 40, 50]));
		let top = list.logical_scroll_top();
		assert_eq!((top.item_ix, top.offset_in_item), (1, px(3.)));
	}

	#[test]
	fn a_list_following_the_newest_message_keeps_following() {
		let list = ListState::new(3, ListAlignment::Bottom, px(120.));
		let mut rows = ids(&[30, 40, 50]);
		splice_rows(&list, &mut rows, ids(&[30, 40, 50, 60]));
		assert_eq!(list.logical_scroll_top().item_ix, 4);
		splice_rows(&list, &mut rows, ids(&[10, 20, 30, 40, 50, 60]));
		assert_eq!(list.logical_scroll_top().item_ix, 6);
	}
}
