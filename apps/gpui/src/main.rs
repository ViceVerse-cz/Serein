//! Experimental native GPUI frontend; transports and secrets remain in Serein's shared crates.
mod backend;
mod input;

use client_core::{Command, Envelope, Event, State};
use gpui::{prelude::*, *};
use model::Id;
use std::time::Duration;

fn color(value: egui::Color32) -> Rgba {
	rgb((u32::from(value.r()) << 16) | (u32::from(value.g()) << 8) | u32::from(value.b()))
}
fn palette() -> ui::design::Palette {
	ui::design::colors(true, ui::design::Variant::Standard)
}

fn tab_navigation(event: &KeyDownEvent, window: &mut Window, cx: &mut App) {
	if event.keystroke.key == "tab" {
		if event.keystroke.modifiers.shift {
			window.focus_prev();
		} else {
			window.focus_next();
		}
		cx.stop_propagation();
	}
}

struct GuildTooltip(SharedString);
impl Render for GuildTooltip {
	fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
		div()
			.p_2()
			.rounded_md()
			.bg(color(palette().raised))
			.text_color(color(palette().text))
			.child(self.0.clone())
	}
}

fn channel_label(channel: &model::Channel) -> String {
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

struct Serein {
	state: State,
	backend: backend::Backend,
	guild: Option<Id>,
	composer: Entity<input::Input>,
	rows: Vec<Id>,
	channels: Vec<Id>,
	messages: ListState,
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
		// A single bounded poll drains at most 32 events; unchanged ticks do not redraw.
		cx.spawn_in(window, async move |this, cx| {
			loop {
				Timer::after(Duration::from_millis(50)).await;
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
		let mut this = Self {
			messages: ListState::new(rows.len(), ListAlignment::Bottom, px(400.)),
			state,
			backend: backend::Backend::start(demo),
			guild,
			composer,
			rows,
			channels: Vec::new(),
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
		this.sync_channels();
		this
	}

	fn poll(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
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
				self.status = "Verifying Discord login…";
				changed = true;
			} else if login.expired() {
				self.login = None;
				self.status = "Login expired. Try again.";
				changed = true;
			}
		}
		let backend_status = *self.backend.status.borrow_and_update();
		if self.backend_status != backend_status {
			self.backend_status = backend_status;
			self.status = backend_status;
			changed = true;
		}
		for _ in 0..32 {
			let Ok(envelope) = self.backend.events.try_recv() else {
				break;
			};
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
		if navigation_changed {
			self.sync_channels();
		}
		if changed {
			if !was_signed_in && self.state.user.is_some() && self.state.selected.is_none() {
				let first = self
					.state
					.channels
					.iter()
					.find(|c| {
						c.supports_text() && !matches!(c.kind, 2 | 13) && self.state.can_view(c.id)
					})
					.map(|c| c.id);
				if let Some(id) = first {
					self.select(id, cx);
				}
			}
			self.sync_rows();
			cx.notify();
		}
	}

	fn sync_channels(&mut self) {
		let mut channels = self
			.state
			.channels
			.iter()
			.filter(|c| {
				c.guild == self.guild
					&& c.supports_text()
					&& !matches!(c.kind, 2 | 13)
					&& self.state.can_view(c.id)
			})
			.collect::<Vec<_>>();
		channels.sort_by_key(|c| (c.position, c.id));
		self.channels = channels.into_iter().map(|c| c.id).collect();
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
			self.messages.splice(
				prefix..self.rows.len() - suffix,
				rows.len() - prefix - suffix,
			);
			self.rows = rows;
		}
	}

	fn select_section(&mut self, guild: Option<Id>, cx: &mut Context<Self>) {
		let channel = self
			.state
			.channels
			.iter()
			.filter(|c| {
				c.guild == guild
					&& c.supports_text()
					&& !matches!(c.kind, 2 | 13)
					&& self.state.can_view(c.id)
			})
			.min_by_key(|c| (c.position, c.id))
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
					..
				} => {
					let mut message =
						test_support::message(10000 + self.state.send_sequence, channel);
					message.content = content;
					message.author = self.state.user.clone().expect("demo user");
					message.nonce = Some(nonce.clone());
					Event::SendResult {
						nonce,
						result: Ok(message),
					}
				}
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
				self.state.status =
					"Draft storage is full; keep or send this draft before switching.";
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
		let command = self.state.select(id);
		self.guild = self.state.channel(id).and_then(|c| c.guild);
		self.sync_channels();
		self.dispatch(command);
		let draft = self.state.drafts.get(&id).cloned().unwrap_or_default();
		self.composer
			.update(cx, |input, cx| input.set_value(draft, cx));
		self.sync_rows();
		if changed {
			self.messages.reset(self.rows.len());
		}
		cx.notify();
	}

	fn send(&mut self, cx: &mut Context<Self>) {
		if !self.save_draft(cx) {
			return;
		}
		let command = self.state.prepare_send();
		if command.is_some() {
			self.dispatch(command);
			self.composer
				.update(cx, |input, cx| input.set_value(String::new(), cx));
			self.sync_rows();
			if !self.rows.is_empty() {
				self.messages.scroll_to_reveal_item(self.rows.len() - 1);
			}
		}
		cx.notify();
	}

	fn message(&self, ix: usize) -> AnyElement {
		let p = palette();
		let Some(message) = self
			.rows
			.get(ix)
			.and_then(|id| self.state.timeline.get_display(*id))
		else {
			return div().into_any_element();
		};
		let name = message.author_nick.as_ref().unwrap_or(&message.author.name);
		div()
			.px_6()
			.py_3()
			.flex()
			.gap_3()
			.w_full()
			.child(
				div()
					.size_9()
					.flex_none()
					.rounded_lg()
					.bg(color(p.raised))
					.flex()
					.items_center()
					.justify_center()
					.text_color(color(p.accent))
					.child(name.chars().take(2).collect::<String>()),
			)
			.child(
				div()
					.flex_1()
					.min_w_0()
					.flex()
					.flex_col()
					.gap_1()
					.child(
						div()
							.text_color(color(p.text_strong))
							.font_weight(FontWeight::SEMIBOLD)
							.child(name.clone())
							.when(message.edited, |d| {
								d.child(div().text_xs().text_color(color(p.muted)).child("edited"))
							}),
					)
					.child(
						div()
							.w_full()
							.whitespace_normal()
							.child(message.display_text().into_owned()),
					)
					.children(message.attachments.iter().map(|a| {
						div()
							.p_2()
							.rounded_md()
							.bg(color(p.raised))
							.child(format!("Attachment · {}", a.filename))
					}))
					.when(!message.embeds.is_empty(), |d| {
						d.child(
							div()
								.text_sm()
								.text_color(color(p.muted))
								.child("Embedded content · open in the main Serein app"),
						)
					}),
			)
			.into_any_element()
	}

	fn guild_row(&self, ix: usize, cx: &mut Context<Self>) -> Stateful<Div> {
		let p = palette();
		let g = &self.state.guilds[ix];
		let id = g.id;
		let name: SharedString = g.name.clone().into();
		div()
			.id(("guild", id.0))
			.focusable()
			.focus(|d| d.border_1().border_color(color(p.accent)))
			.tab_stop(true)
			.w_full()
			.h(px(48.))
			.px_1()
			.py_3()
			.mb_2()
			.rounded_lg()
			.cursor_pointer()
			.bg(color(if self.guild == Some(id) {
				p.selected
			} else {
				p.raised
			}))
			.hover(|d| d.bg(color(p.hover)))
			.text_center()
			.text_sm()
			.tooltip(move |_, cx| cx.new(|_| GuildTooltip(name.clone())).into())
			.child(g.name.chars().take(3).collect::<String>())
			.on_click(cx.listener(move |this, _, _, cx| {
				this.select_section(Some(id), cx);
			}))
	}

	fn channel_row(&self, ix: usize, cx: &mut Context<Self>) -> Stateful<Div> {
		let p = palette();
		let id = self.channels[ix];
		let name = self
			.state
			.channel(id)
			.map(channel_label)
			.unwrap_or_default();
		div()
			.id(("channel", id.0))
			.focusable()
			.focus(|d| d.border_1().border_color(color(p.accent)))
			.tab_stop(true)
			.px_3()
			.h(px(36.))
			.py_2()
			.overflow_hidden()
			.rounded_md()
			.cursor_pointer()
			.bg(color(if self.state.selected == Some(id) {
				p.selected
			} else {
				p.sidebar
			}))
			.hover(|d| d.bg(color(p.hover)))
			.child(format!("# {name}"))
			.on_click(cx.listener(move |this, _, _, cx| this.select(id, cx)))
	}

	fn button(&self, id: &'static str, label: impl Into<SharedString>) -> Stateful<Div> {
		let p = palette();
		div()
			.id(id)
			.focusable()
			.focus(|d| d.border_1().border_color(color(p.accent)))
			.tab_stop(true)
			.px_3()
			.py_2()
			.rounded_md()
			.cursor_pointer()
			.bg(color(p.raised))
			.hover(|d| d.bg(color(p.hover)))
			.child(label.into())
	}
}

impl Render for Serein {
	fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
		let p = palette();
		#[cfg(not(target_os = "linux"))]
		if let Some(login) = &self.login {
			let size = window.viewport_size();
			login.resize_native(f32::from(size.width), f32::from(size.height));
			return div()
				.on_key_down(tab_navigation)
				.size_full()
				.bg(color(p.base))
				.text_color(color(p.text))
				.child(
					div()
						.h(px(56.))
						.px_4()
						.flex()
						.items_center()
						.justify_between()
						.child("discord.com · temporary sign-in window")
						.child(self.button("cancel-login", "Cancel").on_click(cx.listener(
							|this, _, _, cx| {
								this.login = None;
								cx.notify();
							},
						))),
				)
				.into_any_element();
		}
		if self.state.user.is_none() {
			return div()
				.on_key_down(tab_navigation)
				.size_full()
				.bg(color(p.chat))
				.text_color(color(p.text))
				.flex()
				.items_center()
				.justify_center()
				.child(
					div()
						.w(px(460.))
						.p_8()
						.flex()
						.flex_col()
						.gap_4()
						.child(
							div()
								.text_3xl()
								.text_color(color(p.text_strong))
								.child("Serein"),
						)
						.child(div().text_lg().child("GPUI experiment"))
						.child(self.status)
						.child(
							"Your existing Serein login is restored from the OS credential store.",
						)
						.child(
							self.button("retry", "Retry saved login")
								.on_click(cx.listener(|this, _, _, cx| {
									this.backend = backend::Backend::start(false);
									this.status = "Checking saved login…";
									cx.notify();
								})),
						)
						.child(
							self.button(
								"authorize",
								if self.authorized {
									"✓ I own this Discord account"
								} else {
									"□ I own this Discord account"
								},
							)
							.on_click(cx.listener(|this, _, _, cx| {
								this.authorized = !this.authorized;
								cx.notify();
							})),
						)
						.child(
							self.button("login", "Sign in with Discord")
								.on_click(cx.listener(|this, _, window, cx| {
									if !this.authorized {
										this.status =
											"Confirm account ownership before signing in.";
										cx.notify();
										return;
									}
									#[cfg(not(target_os = "linux"))]
									{
										let size = window.viewport_size();
										match platform::LoginView::open_native(
											window,
											f32::from(size.width),
											f32::from(size.height),
											|| {},
										) {
											Ok(login) => {
												this.backend = backend::Backend::start(true);
												this.login = Some(login);
											}
											Err(_) => {
												this.status =
													"The platform login window could not be opened."
											}
										}
									}
									#[cfg(target_os = "linux")]
									{
										let _ = window;
										this.status = "Sign in using the main Serein app, then retry saved login.";
									}
									cx.notify();
								})),
						)
						.child(
							div()
								.text_sm()
								.text_color(color(p.muted))
								.child("Unofficial Discord client · experimental renderer"),
						),
				)
				.into_any_element();
		}

		let guild_name = self
			.guild
			.and_then(|id| self.state.guild(id))
			.map_or("Direct messages".to_owned(), |g| g.name.clone());
		let channel_name = self
			.state
			.selected
			.and_then(|id| self.state.channel(id))
			.map_or("Choose a text channel".to_owned(), |c| {
				format!("# {}", channel_label(c))
			});
		div()
			.on_key_down(tab_navigation)
			.size_full()
			.flex()
			.flex_col()
			.bg(color(p.base))
			.text_color(color(p.text))
			.text_size(px(14.))
			.child(
				div()
					.h(px(36.))
					.flex_none()
					.px_4()
					.flex()
					.items_center()
					.justify_between()
					.text_xs()
					.text_color(color(p.muted))
					.child("SEREIN / GPUI")
					.child(self.status),
			)
			.child(
				div()
					.flex_1()
					.min_h_0()
					.flex()
					.child(
						div()
							.id("guilds")
							.w(px(76.))
							.h_full()
							.flex_none()
							.p_2()
							.flex()
							.flex_col()
							.gap_2()
							.child(self.button("dms", "DM").on_click(cx.listener(
								|this, _, _, cx| {
									this.select_section(None, cx);
								},
							)))
							.child(
								uniform_list(
									"guild-list",
									self.state.guilds.len(),
									cx.processor(|this, range: std::ops::Range<usize>, _, cx| {
										range.map(|ix| this.guild_row(ix, cx)).collect()
									}),
								)
								.flex_1()
								.min_h_0(),
							),
					)
					.child(
						div()
							.w(px(236.))
							.h_full()
							.flex_none()
							.bg(color(p.sidebar))
							.flex()
							.flex_col()
							.child(
								div()
									.h(px(56.))
									.px_4()
									.flex_none()
									.flex()
									.items_center()
									.font_weight(FontWeight::SEMIBOLD)
									.text_color(color(p.text_strong))
									.child(guild_name),
							)
							.child(
								div()
									.px_4()
									.py_2()
									.text_xs()
									.text_color(color(p.muted))
									.child("TEXT CHANNELS"),
							)
							.child(
								uniform_list(
									"channels",
									self.channels.len(),
									cx.processor(|this, range: std::ops::Range<usize>, _, cx| {
										range.map(|ix| this.channel_row(ix, cx)).collect()
									}),
								)
								.flex_1()
								.min_h_0()
								.px_2(),
							)
							.child(
								div().p_4().text_sm().text_color(color(p.muted)).child(
									self.state
										.user
										.as_ref()
										.map_or(String::new(), |u| u.name.clone()),
								),
							),
					)
					.child(
						div()
							.flex_1()
							.min_w_0()
							.h_full()
							.bg(color(p.chat))
							.flex()
							.flex_col()
							.child(
								div()
									.h(px(56.))
									.flex_none()
									.px_6()
									.border_b_1()
									.border_color(color(p.border))
									.flex()
									.items_center()
									.justify_between()
									.child(
										div()
											.font_weight(FontWeight::SEMIBOLD)
											.text_color(color(p.text_strong))
											.child(channel_name),
									)
									.child(self.button("older", "Load older").on_click(
										cx.listener(|this, _, _, cx| {
											let command = this.state.older_history();
											this.dispatch(command);
											this.sync_rows();
											cx.notify();
										}),
									)),
							)
							.child(
								div()
									.flex_1()
									.min_h_0()
									.when(self.rows.is_empty(), |d| {
										d.child(div().p_6().text_color(color(p.muted)).child(
											if self.state.history_pending {
												"Loading messages…"
											} else {
												"No messages to display."
											},
										))
									})
									.child(
										list(
											self.messages.clone(),
											cx.processor(|this, ix, _, _| this.message(ix)),
										)
										.size_full(),
									),
							)
							.children(
								self.state
									.pending
									.iter()
									.filter(|pending| Some(pending.channel) == self.state.selected)
									.map(|pending| {
										div().px_6().py_1().text_color(color(p.muted)).child(
											format!("{:?} · {}", pending.delivery, pending.content),
										)
									}),
							)
							.child(
								div()
									.px_6()
									.pt_2()
									.text_xs()
									.text_color(color(p.muted))
									.child(self.state.status),
							)
							.child(
								div()
									.p_4()
									.flex()
									.items_center()
									.gap_3()
									.child(div().flex_1().min_w_0().child(self.composer.clone()))
									.child(
										self.button("send", "Send")
											.on_click(cx.listener(|this, _, _, cx| this.send(cx))),
									),
							)
							.child(
								div()
									.px_6()
									.pb_3()
									.text_xs()
									.text_color(color(p.muted))
									.child(
										"Enter to send · drafts stay in this experiment’s memory",
									),
							),
					),
			)
			.into_any_element()
	}
}

fn main() {
	let demo = std::env::args().any(|arg| arg == "--demo");
	Application::new().run(move |cx: &mut App| {
		input::init(cx);
		cx.on_window_closed(|cx| {
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
					title: Some("Serein · GPUI experiment".into()),
					..Default::default()
				}),
				..Default::default()
			},
			|window, cx| cx.new(|cx| Serein::new(demo, window, cx)),
		);
		if result.is_err() {
			eprintln!("Could not open the GPUI window.");
			cx.quit();
		}
		cx.activate(true);
	});
}
