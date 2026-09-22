//! Forum containers: a searchable post list with Discord-style cards and one-post creation.
use crate::{design, icons};
use client_core::{Command, MAX_CONTENT, State, forum::MAX_TITLE};
use egui::RichText;
use model::{Channel, Id, archives::Kind};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
enum Sort {
	#[default]
	Activity,
	Created,
}
impl Sort {
	fn label(self) -> &'static str {
		match self {
			Sort::Activity => "Recent activity",
			Sort::Created => "Creation date",
		}
	}
}

#[derive(Default)]
struct Draft {
	title: String,
	body: String,
	focus: bool,
	submitted: bool,
}

#[derive(Default)]
pub struct ForumUi {
	forum: Option<Id>,
	query: String,
	sort: Sort,
	draft: Option<Draft>,
	emoji: crate::emoji_picker::Picker,
}

/// Files chosen for the post's first message. The selection itself lives in the messaging
/// view's upload tray, so a forum never keeps a second copy of anything the user picked.
pub struct Staged<'a> {
	pub files: &'a [(String, u64)],
	pub textures: &'a [Option<egui::TextureHandle>],
	/// Set to ask the desktop shell for the native file chooser.
	pub choose: &'a mut bool,
	/// Set to the index of a card the user removed.
	pub remove: &'a mut Option<usize>,
	/// Set to drop the whole selection, as discarding the draft does.
	pub clear: &'a mut bool,
	pub busy: bool,
}

/// Post open target: an active thread, or an archived row admitted through the archive view.
enum Open {
	Active(Id),
	Archived(Id),
}

impl ForumUi {
	pub fn show(
		&mut self,
		ui: &mut egui::Ui,
		state: &mut State,
		forum: Id,
		commands: &mut Vec<Command>,
		(session, staged): (&mut crate::scroll::Session, &mut Staged<'_>),
		(menu, view): (
			&mut crate::channel_menu::ChannelMenu,
			crate::shortcuts::ShortcutView<'_>,
		),
	) {
		if self.forum != Some(forum) {
			self.forum = Some(forum);
			self.query.clear();
			self.discard_draft(staged);
		}
		if let Some(draft) = &self.draft
			&& draft.submitted
			&& state.posting.pending.is_none()
			&& state.posting.error.is_none()
		{
			self.draft = None;
		}
		if self.draft.is_some()
			&& ui
				.ctx()
				.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
		{
			self.discard_draft(staged);
		}
		if let Some(command) = state.request_forum_posts(forum, false) {
			commands.push(command);
		}
		let colors = design::palette(ui);
		let mut open = None;
		let mut archive_request = None;
		let mut posts_request = false;
		let mut summary_requests = Vec::new();
		let mut author_lookup = Vec::new();
		session
			.attach(
				ui,
				("forum", forum),
				egui::ScrollArea::vertical().auto_shrink([false, false]),
			)
			.show(ui, |ui| {
				egui::Frame::new()
					.inner_margin(egui::Margin::symmetric(16, 12))
					.show(ui, |ui| {
						ui.set_width(ui.available_width());
						ui.spacing_mut().item_spacing.y = 12.0;
						self.toolbar(ui, state, forum);
						if self.draft.is_some() {
							self.composer(ui, state, forum, commands, staged);
						}
						self.sort_menu(ui);
						let query = self.query.trim().to_lowercase();
						let matches = |post: &Channel| {
							query.is_empty() || post.name.to_lowercase().contains(&query)
						};
						let mut posts: Vec<&Channel> = state.forum_posts(forum);
						if self.sort == Sort::Created {
							posts.sort_by_key(|post| std::cmp::Reverse(post.id));
						}
						let active: Vec<_> =
							posts.into_iter().filter(|post| matches(post)).collect();
						let archive = state
							.archives
							.as_ref()
							.filter(|view| view.parent == forum && view.kind == Kind::Public);
						let archived: Vec<&Channel> = archive
							.and_then(|view| view.page.as_ref())
							.map(|page| page.threads.iter().filter(|post| matches(post)).collect())
							.unwrap_or_default();
						let now = time::OffsetDateTime::now_utc();
						let loading = state.posts.parent == Some(forum) && state.posts.loading;
						if active.is_empty() && archived.is_empty() && !loading {
							ui.add_space(24.0);
							ui.vertical_centered(|ui| {
								ui.label(
									design::semibold(
										ui,
										if query.is_empty() {
											"No posts loaded"
										} else {
											"No posts match"
										},
										16.0,
									)
									.color(colors.text_strong),
								);
								ui.label(
									RichText::new(if query.is_empty() {
										"Nothing is posted here yet; archived posts load on request."
									} else {
										"Press Enter to start a post with this title."
									})
									.color(colors.muted),
								);
							});
						}
						for post in active {
							if archived.iter().any(|row| row.id == post.id) {
								continue;
							}
							let response =
								card(ui, state, post, (false, state.post_unread(post)), now);
							if summary_requests.len() < client_core::forum::SUMMARY_BATCH
								&& ui.is_rect_visible(response.rect)
								&& state.needs_post_summary(post.id)
							{
								summary_requests.push(post.id);
							}
							if let Some(latest) = state
								.post_summary(post.id)
								.and_then(|summary| summary.latest.as_ref())
								&& !latest.webhook && latest.roles.is_empty()
								&& author_lookup.len() < client_core::member_search::LIMIT
								&& !author_lookup.contains(&latest.author_id)
							{
								author_lookup.push(latest.author_id);
							}
							menu.context(&response, state, post, view);
							if response.clicked() {
								open = Some(Open::Active(post.id));
							}
						}
						posts_request = posts_footer(ui, state, forum);
						for post in archived {
							let response =
								card(ui, state, post, (true, state.post_unread(post)), now);
							if summary_requests.len() < client_core::forum::SUMMARY_BATCH
								&& ui.is_rect_visible(response.rect)
								&& state.needs_post_summary(post.id)
							{
								summary_requests.push(post.id);
							}
							menu.context(&response, state, post, view);
							if response.clicked() {
								open = Some(Open::Archived(post.id));
							}
						}
						ui.add_space(4.0);
						archive_request = archive_footer(ui, state, forum, archive);
					});
			});
		if open.is_none()
			&& let Some(command) = state.request_post_summaries(summary_requests)
		{
			commands.push(command);
		}
		if let Some(command) = state.request_author_members(&author_lookup) {
			commands.push(command);
		}
		if let Some(open) = open {
			match open {
				Open::Active(id) => {
					if let Some(command) = state.select(id) {
						commands.push(command);
					}
				}
				Open::Archived(id) => {
					if let Some(command) = state.open_archived_thread(id) {
						commands.push(command);
					}
				}
			}
		} else if let Some(before) = archive_request
			&& let Some(command) = state.request_archives(forum, Kind::Public, before)
		{
			commands.push(command);
		} else if posts_request {
			state.posts.error = None;
			if let Some(command) = state.request_forum_posts(forum, state.posts.loaded > 0) {
				commands.push(command);
			}
		}
	}

	fn toolbar(&mut self, ui: &mut egui::Ui, state: &State, forum: Id) {
		let colors = design::palette(ui);
		egui::Frame::new()
			.fill(colors.raised)
			.stroke(egui::Stroke::new(1.0, colors.border))
			.corner_radius(8)
			.inner_margin(egui::Margin::symmetric(12, 8))
			.show(ui, |ui| {
				ui.set_width(ui.available_width());
				ui.horizontal(|ui| {
					ui.spacing_mut().item_spacing.x = 8.0;
					let allowed = state.can_create_post(forum);
					ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
						let button = ui.add_enabled(
							allowed && self.draft.is_none(),
							egui::Button::new(
								design::medium(ui, "New Post", 14.0).color(colors.accent_text),
							)
							.fill(colors.accent)
							.stroke(egui::Stroke::NONE)
							.corner_radius(8)
							.min_size(egui::vec2(0.0, 32.0)),
						);
						if button.clicked() {
							self.start_draft(String::new());
						}
						if !allowed && state.is_forum(forum) {
							button.on_disabled_hover_text(
								"Posting requires a connected session with permission to send here.",
							);
						}
						ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
							icons::inline(ui, icons::Icon::Search, 20.0, colors.muted);
							let input = ui.add(
								egui::TextEdit::singleline(&mut self.query)
									.char_limit(MAX_TITLE)
									.frame(egui::Frame::NONE)
									.hint_text("Search or create a post...")
									.font(egui::TextStyle::Body)
									.desired_width(ui.available_width().max(60.0)),
							);
							let input =
								input.accessible_name("Search loaded posts or start a new one");
							if input.lost_focus()
								&& ui.input(|i| i.key_pressed(egui::Key::Enter))
								&& !self.query.trim().is_empty()
								&& allowed
							{
								let title = std::mem::take(&mut self.query);
								self.start_draft(title);
							}
						});
					});
				});
			});
	}

	fn start_draft(&mut self, title: String) {
		self.draft = Some(Draft {
			title: title.trim().chars().take(MAX_TITLE).collect(),
			body: String::new(),
			focus: true,
			submitted: false,
		});
	}

	fn sort_menu(&mut self, ui: &mut egui::Ui) {
		let colors = design::palette(ui);
		let button = ui.add(
			egui::Button::new(
				design::medium(ui, format!("Sort & view · {}", self.sort.label()), 13.0)
					.color(colors.text),
			)
			.fill(colors.raised)
			.stroke(egui::Stroke::new(1.0, colors.border))
			.corner_radius(8)
			.min_size(egui::vec2(0.0, 30.0)),
		);
		egui::Popup::menu(&button).show(|ui| {
			ui.set_min_width(180.0);
			ui.label(design::eyebrow(ui, "Sort by", colors.muted));
			for sort in [Sort::Activity, Sort::Created] {
				if ui.radio(self.sort == sort, sort.label()).clicked() {
					self.sort = sort;
				}
			}
			ui.separator();
			ui.label(design::eyebrow(ui, "View", colors.muted));
			ui.add_enabled(false, egui::Button::selectable(true, "List view"));
		});
	}

	/// Drop the draft and anything staged with it; a discarded post keeps no selection.
	fn discard_draft(&mut self, staged: &mut Staged<'_>) {
		if self.draft.take().is_some() && !staged.files.is_empty() {
			*staged.clear = true;
		}
	}

	/// Discord's post composer: one card holding the title, the first message, its images and
	/// the actions that send them together.
	fn composer(
		&mut self,
		ui: &mut egui::Ui,
		state: &mut State,
		forum: Id,
		commands: &mut Vec<Command>,
		staged: &mut Staged<'_>,
	) {
		let colors = design::palette(ui);
		let files_allowed = state.can_attach_post(forum);
		let posting = state.posting.pending.is_some() || staged.busy;
		let error = state.posting.error;
		let emoji = &mut self.emoji;
		let Some(draft) = self.draft.as_mut() else {
			return;
		};
		let mut submit = false;
		let mut cancel = false;
		egui::Frame::new()
			.fill(colors.raised)
			.stroke(egui::Stroke::new(1.0, colors.border))
			.corner_radius(12)
			.show(ui, |ui| {
				ui.set_width(ui.available_width());
				ui.spacing_mut().item_spacing.y = 0.0;
				egui::Frame::new()
					.inner_margin(egui::Margin::symmetric(16, 14))
					.show(ui, |ui| {
						ui.set_width(ui.available_width());
						ui.horizontal_top(|ui| {
							ui.spacing_mut().item_spacing.x = 10.0;
							if icons::button(ui, icons::Icon::Close, 22.0, "Discard this post")
								.clicked()
							{
								cancel = true;
							}
							const THUMB: f32 = 72.0;
							let fields = (ui.available_width() - THUMB - 10.0).max(160.0);
							ui.vertical(|ui| {
								ui.set_width(fields);
								ui.spacing_mut().item_spacing.y = 4.0;
								let title = ui.add(
									egui::TextEdit::singleline(&mut draft.title)
										.char_limit(MAX_TITLE)
										.frame(egui::Frame::NONE)
										.font(egui::FontId::new(
											20.0,
											design::semibold_family(ui.ctx()),
										))
										.text_color(colors.text_strong)
										.hint_text(
											design::semibold(ui, "Title", 20.0).color(colors.muted),
										)
										.desired_width(f32::INFINITY),
								);
								let title = title.accessible_name("Post title");
								if draft.focus {
									title.request_focus();
									draft.focus = false;
								}
								let body = ui.add(
									egui::TextEdit::multiline(&mut draft.body)
										.char_limit(MAX_CONTENT)
										.frame(egui::Frame::NONE)
										.hint_text(
											RichText::new("Enter a message...")
												.size(15.0)
												.color(colors.muted),
										)
										.desired_rows(3)
										.desired_width(f32::INFINITY),
								);
								body.accessible_name("First message of this post");
							});
							// Discord parks the image control beside the fields, not under them.
							ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
								let enabled = files_allowed
									&& !posting && staged.files.len()
									< client_core::MAX_ATTACHMENTS;
								let (rect, response) = ui.allocate_exact_size(
									egui::Vec2::splat(THUMB),
									if enabled {
										egui::Sense::click()
									} else {
										egui::Sense::hover()
									},
								);
								let hovered =
									enabled && (response.hovered() || response.has_focus());
								ui.painter().rect(
									rect,
									10,
									if hovered {
										colors.hover
									} else {
										colors.sidebar
									},
									egui::Stroke::new(1.0, colors.border),
									egui::StrokeKind::Inside,
								);
								icons::paint(
									ui.painter(),
									icons::Icon::Image,
									egui::Rect::from_center_size(
										rect.center(),
										egui::Vec2::splat(30.0),
									),
									if !enabled {
										colors.muted.gamma_multiply(0.5)
									} else if hovered {
										colors.text_strong
									} else {
										colors.text
									},
								);
								response.widget_info(|| {
									egui::WidgetInfo::labeled(
										egui::Role::Button,
										enabled,
										"Add images to this post",
									)
								});
								if response.clicked() {
									*staged.choose = true;
								}
								response.on_hover_text(if files_allowed {
									"Add images or files. Up to 10 files and 500 MB total; account limits may be lower."
								} else {
									"Attaching files is unavailable in this forum."
								});
							});
						});
						if !staged.files.is_empty() {
							ui.add_space(10.0);
							tray(ui, staged);
						}
					});
				let (rule, _) = ui.allocate_exact_size(
					egui::vec2(ui.available_width(), 1.0),
					egui::Sense::hover(),
				);
				ui.painter().rect_filled(rule, 0, colors.border);
				egui::Frame::new()
					.inner_margin(egui::Margin::symmetric(12, 10))
					.show(ui, |ui| {
						ui.set_width(ui.available_width());
						ui.horizontal(|ui| {
							ui.spacing_mut().item_spacing.x = 8.0;
							let mut inserted = None;
							emoji.unicode_button_with(ui, &mut inserted, false);
							if let Some(text) = inserted {
								// This composer tracks no caret, so a pick lands at the end.
								crate::emoji_picker::insert(
									&mut draft.body,
									&text,
									None,
									MAX_CONTENT,
								);
							}
							ui.with_layout(
								egui::Layout::right_to_left(egui::Align::Center),
								|ui| {
									let ready = state.can_create_post(forum)
										&& !draft.title.trim().is_empty()
										&& (!draft.body.trim().is_empty()
											|| !staged.files.is_empty());
									let post = ui.add_enabled(
										ready && !draft.submitted && !posting,
										egui::Button::new(
											design::medium(ui, "Post", 14.0)
												.color(colors.accent_text),
										)
										.fill(colors.accent)
										.stroke(egui::Stroke::NONE)
										.corner_radius(8)
										.min_size(egui::vec2(96.0, 34.0)),
									);
									submit = post.clicked();
									if posting {
										ui.label(RichText::new("Posting…").color(colors.muted));
									} else if let Some(error) = error {
										ui.label(RichText::new(error).color(colors.danger));
									}
									ui.label(
										RichText::new(format!(
											"{}/{MAX_TITLE} · {}/{MAX_CONTENT}",
											draft.title.chars().count(),
											draft.body.chars().count()
										))
										.size(11.0)
										.color(colors.muted),
									);
								},
							);
						});
					});
			});
		if cancel {
			self.discard_draft(staged);
			state.posting.error = None;
		} else if submit {
			let names: Vec<&str> = staged.files.iter().map(|(name, _)| name.as_str()).collect();
			if let Some(command) =
				state.create_post_with_attachments(forum, &draft.title, &draft.body, &names)
			{
				draft.submitted = true;
				commands.push(command);
			}
		}
	}
}

/// Chosen files above the actions, in the same cards the message composer uses.
fn tray(ui: &mut egui::Ui, staged: &mut Staged<'_>) {
	egui::ScrollArea::horizontal()
		.id_salt("forum-post-attachments")
		.show(ui, |ui| {
			ui.horizontal(|ui| {
				ui.spacing_mut().item_spacing.x = 12.0;
				for (index, (filename, bytes)) in staged.files.iter().enumerate() {
					ui.push_id(index, |ui| {
						if crate::attachments::pending_card(
							ui,
							filename,
							*bytes,
							staged.textures.get(index).and_then(Option::as_ref),
							!staged.busy,
						) {
							*staged.remove = Some(index);
						}
					});
				}
			});
		});
}

fn card(
	ui: &mut egui::Ui,
	state: &State,
	post: &Channel,
	(archived, unread): (bool, bool),
	now: time::OffsetDateTime,
) -> egui::Response {
	let colors = design::palette(ui);
	let response = ui
		.scope_builder(
			egui::UiBuilder::new()
				.id_salt(("post", post.id, archived))
				.sense(egui::Sense::click()),
			|ui| {
				let response = ui.response();
				design::interactive_card_frame(ui, &response)
					.inner_margin(egui::Margin::symmetric(16, 14))
					.show(ui, |ui| {
						ui.set_width(ui.available_width());
						ui.spacing_mut().item_spacing.y = 6.0;
						// A read post keeps its title quiet; only unread ones stay bright.
						ui.horizontal(|ui| {
							ui.spacing_mut().item_spacing.x = 8.0;
							if unread {
								let (rect, _) = ui.allocate_exact_size(
									egui::vec2(8.0, 8.0),
									egui::Sense::hover(),
								);
								ui.painter()
									.circle_filled(rect.center(), 4.0, colors.text_strong);
							}
							ui.add(
								egui::Label::new(if unread {
									design::semibold(ui, &post.name, 16.0).color(colors.text_strong)
								} else {
									design::medium(ui, &post.name, 16.0).color(colors.muted)
								})
								.truncate()
								.selectable(false),
							);
						});
						let latest = state
							.post_summary(post.id)
							.and_then(|summary| summary.latest.as_ref());
						if let Some(latest) = latest {
							ui.horizontal(|ui| {
								ui.spacing_mut().item_spacing.x = 5.0;
								let author_color = state
									.forum_author_color(
										post.id,
										latest.author_id,
										latest.webhook,
										&latest.roles,
									)
									.map_or(colors.text_strong, |rgb| {
										design::role_name_color(
											rgb,
											colors.raised,
											colors.text_strong,
										)
									});
								ui.label(
									design::semibold(ui, format!("{}:", latest.author), 14.0)
										.color(author_color),
								);
								ui.add(
									egui::Label::new(
										RichText::new(if latest.excerpt.trim().is_empty() {
											"Attachment or non-text message"
										} else {
											&latest.excerpt
										})
										.size(14.0)
										.color(colors.text),
									)
									.truncate()
									.selectable(false),
								);
							});
						} else {
							ui.label(
								RichText::new("Latest message unavailable")
									.size(14.0)
									.color(colors.muted),
							);
						}
						ui.horizontal(|ui| {
							ui.spacing_mut().item_spacing.x = 6.0;
							if let Some(count) = post.message_count {
								icons::inline(ui, icons::Icon::Forum, 16.0, colors.muted);
								ui.label(
									design::medium(ui, count.to_string(), 13.0).color(colors.text),
								);
							}
							if unread {
								let label = match state.post_new_count(post) {
									Some((count, exact)) if count > 0 => {
										format!("({count}{} New)", if exact { "" } else { "+" })
									}
									_ => "(New)".to_owned(),
								};
								ui.label(design::medium(ui, label, 13.0).color(colors.accent));
							}
							ui.label(RichText::new("·").color(colors.muted));
							ui.label(
								RichText::new(ago(post.last_message.unwrap_or(post.id), now))
									.size(13.0)
									.color(colors.muted),
							);
							if archived {
								ui.label(RichText::new("·").color(colors.muted));
								ui.label(RichText::new("Archived").size(13.0).color(colors.muted));
							}
						});
					});
			},
		)
		.response;
	response.widget_info(|| {
		egui::WidgetInfo::labeled(
			egui::Role::Button,
			true,
			format!(
				"{}{}{}; {} replies",
				post.name,
				if unread { ", unread" } else { "" },
				if archived { ", archived" } else { "" },
				post.message_count
					.map_or("unknown".to_owned(), |n| n.to_string())
			),
		)
	});
	response
}

/// Active-post status under the list; returns true when the user asks for another page.
fn posts_footer(ui: &mut egui::Ui, state: &State, forum: Id) -> bool {
	let colors = design::palette(ui);
	if state.posts.parent != Some(forum) {
		return false;
	}
	let mut request = false;
	ui.horizontal_wrapped(|ui| {
		if state.posts.loading {
			ui.label(
				RichText::new("Loading posts…")
					.size(13.0)
					.color(colors.muted),
			);
		} else if let Some(error) = state.posts.error {
			ui.label(RichText::new(error).size(13.0).color(colors.danger));
			request = ui
				.add_enabled(
					state.can_load_posts(forum),
					egui::Button::new(RichText::new("Retry").size(13.0)),
				)
				.clicked();
		} else if state.posts.more {
			request = ui
				.add(
					egui::Button::new(
						RichText::new("Load more posts")
							.size(13.0)
							.color(colors.link),
					)
					.frame(false),
				)
				.clicked();
		}
	});
	request
}

/// Archive controls under the list; returns a page cursor request when the user asks for one.
fn archive_footer(
	ui: &mut egui::Ui,
	state: &State,
	forum: Id,
	view: Option<&client_core::archives::View>,
) -> Option<Option<model::archives::Cursor>> {
	let colors = design::palette(ui);
	let allowed = state.can_archive(forum, Kind::Public);
	let mut request = None;
	ui.horizontal_wrapped(|ui| match view {
		None => {
			let button = ui.add_enabled(
				allowed,
				egui::Button::new(
					RichText::new("Load archived posts")
						.size(13.0)
						.color(colors.link),
				)
				.frame(false),
			);
			if button.clicked() {
				request = Some(None);
			}
			if !allowed {
				ui.label(
					RichText::new("Archived posts need a connected session with history access.")
						.size(12.0)
						.color(colors.muted),
				);
			}
		}
		Some(view) if view.loading => {
			ui.label(
				RichText::new("Loading archived posts…")
					.size(13.0)
					.color(colors.muted),
			);
		}
		Some(view) => {
			if let Some(error) = view.error {
				ui.label(RichText::new(error).size(13.0).color(colors.danger));
				if ui
					.add_enabled(
						allowed,
						egui::Button::new(RichText::new("Retry").size(13.0)),
					)
					.clicked()
				{
					request = Some(view.before);
				}
			} else if let Some(page) = &view.page {
				if let Some(next) = page.next {
					if ui
						.add_enabled(
							allowed,
							egui::Button::new(
								RichText::new("Older archived posts")
									.size(13.0)
									.color(colors.link),
							)
							.frame(false),
						)
						.clicked()
					{
						request = Some(Some(next));
					}
				} else {
					ui.label(
						RichText::new("No older archived posts reported.")
							.size(12.0)
							.color(colors.muted),
					);
				}
			}
		}
	});
	request
}

// Discord snowflakes carry milliseconds since 2015-01-01; all u64 IDs fit time's range.
fn ago(id: Id, now: time::OffsetDateTime) -> String {
	let created =
		time::OffsetDateTime::from_unix_timestamp(((id.0 >> 22) / 1000) as i64 + 1_420_070_400)
			.expect("snowflake timestamp is in range");
	let seconds = (now - created).whole_seconds().max(0);
	match seconds {
		0..60 => "just now".to_owned(),
		60..3_600 => format!("{}m ago", seconds / 60),
		3_600..86_400 => format!("{}h ago", seconds / 3_600),
		86_400..2_592_000 => format!("{}d ago", seconds / 86_400),
		2_592_000..31_536_000 => format!("{}mo ago", seconds / 2_592_000),
		_ => format!("{}y ago", seconds / 31_536_000),
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	/// Nothing is staged in these fixtures; the tray lives in the messaging view.
	#[derive(Default)]
	struct Scratch {
		choose: bool,
		remove: Option<usize>,
		clear: bool,
	}
	fn staged(scratch: &mut Scratch) -> Staged<'_> {
		Staged {
			files: &[],
			textures: &[],
			choose: &mut scratch.choose,
			remove: &mut scratch.remove,
			clear: &mut scratch.clear,
			busy: false,
		}
	}

	fn frame(ctx: &egui::Context, draw: impl FnMut(&mut egui::Ui)) {
		let output = ctx.run_ui(
			egui::RawInput {
				screen_rect: Some(egui::Rect::from_min_size(
					egui::Pos2::ZERO,
					egui::vec2(720.0, 640.0),
				)),
				..Default::default()
			},
			draw,
		);
		output.drop_without_applying_deltas();
	}

	#[test]
	fn relative_times_round_down() {
		let now = time::OffsetDateTime::from_unix_timestamp(1_800_000_000).unwrap();
		let at = |seconds_ago: i64| {
			Id((((1_800_000_000 - seconds_ago - 1_420_070_400) as u64) * 1000) << 22)
		};
		assert_eq!(ago(at(5), now), "just now");
		assert_eq!(ago(at(125), now), "2m ago");
		assert_eq!(ago(at(7_200), now), "2h ago");
		assert_eq!(ago(at(15 * 86_400), now), "15d ago");
		assert_eq!(ago(at(70 * 86_400), now), "2mo ago");
		assert_eq!(ago(at(800 * 86_400), now), "2y ago");
	}

	#[test]
	fn post_composer_stages_images_and_sends_them_with_the_first_message() {
		let ctx = egui::Context::default();
		let mut state = test_support::demo_state();
		state.gateway_connected = true;
		state.auth = client_core::auth::AuthState::Authenticated;
		assert!(state.select(Id(26)).is_none());
		assert!(
			state.can_attach_post(Id(26)),
			"the fixture forum allows files"
		);
		let mut forum = ForumUi::default();
		let mut commands = Vec::new();
		let mut scratch = Scratch::default();
		let files = [("synthetic.png".to_owned(), 2_048)];
		let textures = [None];
		let render = |forum: &mut ForumUi,
		              state: &mut client_core::State,
		              commands: &mut Vec<Command>,
		              scratch: &mut Scratch| {
			let output = ctx.run_ui(
				egui::RawInput {
					screen_rect: Some(egui::Rect::from_min_size(
						egui::Pos2::ZERO,
						egui::vec2(720.0, 640.0),
					)),
					..Default::default()
				},
				|ui| {
					let mut staged = Staged {
						files: &files,
						textures: &textures,
						choose: &mut scratch.choose,
						remove: &mut scratch.remove,
						clear: &mut scratch.clear,
						busy: false,
					};
					forum.show(
						ui,
						state,
						Id(26),
						commands,
						(&mut crate::scroll::Session::default(), &mut staged),
						(
							&mut crate::channel_menu::ChannelMenu::default(),
							crate::shortcuts::ShortcutView::new(&Default::default(), true),
						),
					);
				},
			);
			output.drop_without_applying_deltas();
		};
		render(&mut forum, &mut state, &mut commands, &mut scratch);
		forum.start_draft("Roadmap ideas".into());
		render(&mut forum, &mut state, &mut commands, &mut scratch);
		assert!(commands.is_empty(), "rendering never posts by itself");
		// An image without text is still a post; Discord accepts an empty starter body.
		let draft = forum.draft.as_ref().expect("composer stays open");
		let command = state
			.create_post_with_attachments(Id(26), &draft.title, &draft.body, &["synthetic.png"])
			.expect("staged files travel with the post");
		let Command::CreatePost {
			parent,
			attachments,
			..
		} = &command
		else {
			panic!("post creation command expected");
		};
		assert_eq!(
			(*parent, attachments.as_slice()),
			(Id(26), &["synthetic.png".to_owned()][..])
		);
		// Discarding the draft releases the selection instead of leaving it staged.
		state.posting.pending = None;
		forum.discard_draft(&mut staged(&mut scratch));
		assert!(forum.draft.is_none());
	}

	#[test]
	fn forum_pane_lists_posts_and_creates_then_opens_one() {
		for dark in [false, true] {
			let ctx = egui::Context::default();
			ctx.set_visuals(if dark {
				egui::Visuals::dark()
			} else {
				egui::Visuals::light()
			});
			let mut state = test_support::demo_state();
			state.gateway_connected = true;
			state.auth = client_core::auth::AuthState::Authenticated;
			assert!(state.select(Id(26)).is_none());
			assert!(state.is_forum(Id(26)));
			let posts = state.forum_posts(Id(26));
			assert!(posts.len() >= 3, "fixture ships several posts");
			assert!(posts.iter().all(|post| post.parent_id == Some(Id(26))));
			let mut forum = ForumUi::default();
			let mut scratch = Scratch::default();
			let mut commands = Vec::new();
			frame(&ctx, |ui| {
				forum.show(
					ui,
					&mut state,
					Id(26),
					&mut commands,
					(
						&mut crate::scroll::Session::default(),
						&mut staged(&mut scratch),
					),
					(
						&mut crate::channel_menu::ChannelMenu::default(),
						crate::shortcuts::ShortcutView::new(&Default::default(), true),
					),
				)
			});
			assert!(
				commands.is_empty(),
				"Rendering never requests history or archives"
			);
			forum.query = "synthetic".into();
			frame(&ctx, |ui| {
				forum.show(
					ui,
					&mut state,
					Id(26),
					&mut commands,
					(
						&mut crate::scroll::Session::default(),
						&mut staged(&mut scratch),
					),
					(
						&mut crate::channel_menu::ChannelMenu::default(),
						crate::shortcuts::ShortcutView::new(&Default::default(), true),
					),
				)
			});
			assert!(commands.is_empty());
			forum.start_draft("Roadmap ideas".into());
			forum.draft.as_mut().unwrap().body = "First message".into();
			frame(&ctx, |ui| {
				forum.show(
					ui,
					&mut state,
					Id(26),
					&mut commands,
					(
						&mut crate::scroll::Session::default(),
						&mut staged(&mut scratch),
					),
					(
						&mut crate::channel_menu::ChannelMenu::default(),
						crate::shortcuts::ShortcutView::new(&Default::default(), true),
					),
				)
			});
			let draft = forum.draft.as_mut().unwrap();
			let command = state
				.create_post(Id(26), &draft.title, &draft.body)
				.expect("fixture permissions allow posting");
			draft.submitted = true;
			let Command::CreatePost {
				parent,
				request,
				title,
				..
			} = &command
			else {
				panic!("post creation command expected");
			};
			assert_eq!((*parent, title.as_str()), (Id(26), "Roadmap ideas"));
			state.apply_post(
				Id(26),
				*request,
				Ok(Channel {
					id: Id(1_548_000_000_000_000_000),
					guild: Some(Id(10)),
					parent_id: Some(Id(26)),
					position: 0,
					name: title.clone(),
					kind: 11,
					recipients: vec![],
					last_message: None,
					member_list_id: None,
					message_count: Some(0),
					icon: None,
				}),
			);
			assert_eq!(state.posting.created, Some(Id(1_548_000_000_000_000_000)));
			frame(&ctx, |ui| {
				forum.show(
					ui,
					&mut state,
					Id(26),
					&mut commands,
					(
						&mut crate::scroll::Session::default(),
						&mut staged(&mut scratch),
					),
					(
						&mut crate::channel_menu::ChannelMenu::default(),
						crate::shortcuts::ShortcutView::new(&Default::default(), true),
					),
				)
			});
			assert!(
				forum.draft.is_none(),
				"A confirmed post closes the composer"
			);
			assert!(
				commands.is_empty(),
				"Opening the created post is the layout's job"
			);
			assert_eq!(
				state.forum_posts(Id(26))[0].id,
				Id(1_548_000_000_000_000_000)
			);
			assert!(matches!(
				state.select(Id(1_548_000_000_000_000_000)),
				Some(Command::History {
					channel: Id(1_548_000_000_000_000_000),
					..
				})
			));
			// A live session fetches the posts the gateway never delivered, exactly once.
			state.demo = false;
			let mut forum = ForumUi::default();
			frame(&ctx, |ui| {
				forum.show(
					ui,
					&mut state,
					Id(26),
					&mut commands,
					(
						&mut crate::scroll::Session::default(),
						&mut staged(&mut scratch),
					),
					(
						&mut crate::channel_menu::ChannelMenu::default(),
						crate::shortcuts::ShortcutView::new(&Default::default(), true),
					),
				)
			});
			let Some(Command::ForumPosts {
				parent: Id(26),
				offset: 0,
				request,
				..
			}) = commands.pop()
			else {
				panic!("the post list loads itself");
			};
			assert!(commands.is_empty());
			frame(&ctx, |ui| {
				forum.show(
					ui,
					&mut state,
					Id(26),
					&mut commands,
					(
						&mut crate::scroll::Session::default(),
						&mut staged(&mut scratch),
					),
					(
						&mut crate::channel_menu::ChannelMenu::default(),
						crate::shortcuts::ShortcutView::new(&Default::default(), true),
					),
				)
			});
			assert!(commands.is_empty(), "A pending page is never re-requested");
			state.apply_forum_posts(
				Id(26),
				request,
				Ok(model::forum::Page {
					threads: vec![Channel {
						id: Id(1_549_000_000_000_000_000),
						guild: Some(Id(10)),
						parent_id: Some(Id(26)),
						position: 0,
						name: "Fetched post".into(),
						kind: 11,
						recipients: vec![],
						last_message: None,
						member_list_id: None,
						message_count: Some(2),
						icon: None,
					}],
					more: false,
				}),
			);
			forum.query.clear();
			frame(&ctx, |ui| {
				forum.show(
					ui,
					&mut state,
					Id(26),
					&mut commands,
					(
						&mut crate::scroll::Session::default(),
						&mut staged(&mut scratch),
					),
					(
						&mut crate::channel_menu::ChannelMenu::default(),
						crate::shortcuts::ShortcutView::new(&Default::default(), true),
					),
				)
			});
			assert!(commands.is_empty(), "A loaded forum stays quiet");
			assert!(
				state
					.forum_posts(Id(26))
					.iter()
					.any(|post| post.id == Id(1_549_000_000_000_000_000)),
				"Fetched posts join the list"
			);
		}
	}
}
