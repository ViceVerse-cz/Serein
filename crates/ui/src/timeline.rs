use crate::design::LazyHover;
use crate::markdown::{FormatCache, Formatted, discord_url};
use client_core::State;
use egui::RichText;
use model::{Id, Message};
use std::{
	collections::{BTreeMap, BTreeSet},
	hash::{DefaultHasher, Hash, Hasher},
};

#[derive(Clone, Copy)]
enum TargetReveal {
	StayIfVisible,
	Center,
}

const REVEAL_SCROLL_SECS: f32 = 0.3;
/// Smooth return to the live edge from the jump-to-present control.
const PRESENT_SCROLL_SECS: f32 = 0.32;
/// Screens of history a reader must leave behind before the control appears.
const PRESENT_CONTROL_SCREENS: f32 = 6.0;

#[derive(Clone, Copy)]
struct RevealScroll {
	target: Id,
	from: f32,
	elapsed: f32,
}

#[derive(Default)]
pub struct TimelineView {
	polls: crate::polls::Cards,
	pub(super) poll_action: Option<(Id, client_core::polls::Action)>,
	pub(super) forward_request: Option<Id>,
	pub(super) sticker_request: Option<Id>,
	pub(super) browse_sticker: Option<model::Sticker>,
	pub(super) component_viewing: Option<(Id, u64)>,
	pub(super) components: crate::components::Components,
	pub(super) component_action: Option<crate::components::Action>,
	/// A private command response the reader dismissed this frame.
	pub(super) dismiss_ephemeral: Option<Id>,
	pub(super) extension_actions: std::sync::Arc<Vec<crate::extensions_ui::MenuAction>>,
	pub(super) extension_request: Option<(crate::extensions_ui::MenuAction, String)>,
	pub(super) user_action: Option<crate::user_menu::Action>,
	pub(super) restore_pending: Option<String>,
	pub(super) cancel_upload: bool,
	pending_heights: BTreeMap<String, f32>,
	pub(super) hide_media_links: bool,
	pub(super) instant_scrolling: bool,
	applied_hide_media_links: bool,
	pub(super) compact_messages: bool,
	applied_compact_messages: bool,
	pub(super) gif_favorite: Option<model::Gif>,
	pub(super) invite_requests: Vec<String>,
	pub(super) invite_action: Option<crate::invites::Action>,
	pub(super) edit_started: bool,
	pub(super) reply_started: bool,
	pub(super) quick_delete: Option<(Id, Id)>,
	pub(super) channel_reference: Option<Id>,
	pub(super) channel_reference_load: Option<Id>,
	pub(super) pending_channel_reference: Option<Id>,
	pub(super) reply_target: Option<Id>,
	pending_reveal: Option<TargetReveal>,
	reveal_scroll: Option<RevealScroll>,
	/// Animated return to the live edge: offset it started from and elapsed seconds.
	present_scroll: Option<(f32, f32)>,
	/// Where the jump-to-present control was painted this frame, for hit tests in tests.
	pub(super) present_control: Option<egui::Rect>,
	highlighted: Option<(Id, f64)>,
	target_browsing: bool,
	hold_read_ack: bool,
	initial_read_checked: bool,
	pub(super) unread_jump: bool,
	pub(super) load_newer: bool,
	pub(super) mark_read: Option<Id>,
	pub(super) mark_unread: Option<Id>,
	pub(super) auto_read_attempt: Option<Id>,
	at_current_latest: bool,
	/// The unread banner was raised during this visit; it stays until the reader leaves.
	unread_session: bool,
	/// Newest live-edge message the reader had on screen at the bottom during this visit.
	seen_latest: Option<Id>,
	/// Channel left this frame and the message to acknowledge there.
	pub(super) leave_read: Option<(Id, Id)>,
	/// The reader dismissed the unread banner while this was the channel's latest message.
	unread_dismissed: Option<Option<Id>>,
	/// The unread banner asked to acknowledge this channel up to its latest message.
	pub(super) mark_channel_read: Option<Id>,
	pub(super) reaction_picker: Option<(Id, egui::Rect, egui::Id)>,
	pub(super) reaction: Option<(Id, Option<model::ReactionEmoji>)>,
	pub(super) reaction_users: Option<(Id, model::ReactionEmoji, bool)>,
	/// Hover-bar reactions, most used first; `None` uses the defaults.
	pub(super) quick_reactions: Option<[&'static str; 3]>,
	/// Explicit preset for the double-click gesture; `None` disables it.
	pub(super) double_click_reaction: Option<&'static str>,
	/// A quick reaction the reader added this frame, for the usage ranking.
	pub(super) quick_reaction_used: Option<&'static str>,
	/// Requested pin change: channel, message, pinned.
	pub(super) pin_request: Option<(Id, Id, bool)>,
	/// Requested new thread: parent channel and the message that starts it.
	pub(super) thread_request: Option<(Id, Id)>,
	/// Channel whose Threads dialog a system row asked to open.
	pub(super) threads_request: Option<Id>,
	/// The starter message shown as a pseudo-row at the top of an exhausted thread.
	starter_row: Option<Id>,
	suppressed_deleted_highlight: BTreeSet<Id>,
	pub(super) remove_preserved: Option<Id>,
	toolbar: Option<(Id, egui::Rect)>,
	copy_feedback: Option<(egui::Id, f64)>,
	heights: BTreeMap<Id, (u64, f32)>,
	// Heights can remain resize estimates; only these bounded active-row IDs were
	// measured with the current dimensions and state revision.
	measured_rows: BTreeSet<Id>,
	#[cfg(test)]
	leading_rendered: usize,
	pub(super) reflow_frames: u64,
	pub(super) consecutive_reflows: u64,
	width: f32,
	rows: Vec<(Id, f32)>,
	revision: u64,
	layout_fingerprint: u64,
	channel: Option<Id>,
	anchor: Option<(Id, f32)>,
	scroll_offset: f32,
	following: bool,
	formatted: FormatCache,
	pending_formatted: FormatCache,
	// A fingerprint of the revealed content prevents a reload that resets model revisions from
	// revealing edits, without cloning payloads. Pruned with the active window: at most 500 records.
	revealed: BTreeMap<Id, Revealed>,
	pub(super) viewing: Option<(Id, Id)>,
	embed_viewing: Option<(Id, u64, model::Attachment)>,
	/// Fixture-only: viewer to open once its message has arrived in the timeline.
	pending_viewer: Option<(Id, Id)>,
	pub(super) download: crate::attachments::DownloadUi,
	pub(super) audio: crate::audio::AudioUi,
	pub(super) video: crate::video::VideoUi,
	pub(super) opening: Option<String>,
	pub(super) browser_opening: Option<String>,
	text_size: f32,
	font_revision: (usize, usize, u32),
	scale: f32,
	pub(super) load_older: bool,
	pub(super) latest: bool,
	pub(super) visible_authors: Vec<Id>,
	jump: bool,
	unread_boundary: Option<Id>,
}
struct Revealed {
	/// Fingerprint of the content, embeds and attachments that were revealed.
	content: u64,
	text: u32,
	media: bool,
}
impl Revealed {
	fn fingerprint(message: &Message) -> u64 {
		let mut hasher = DefaultHasher::new();
		message.content.hash(&mut hasher);
		message.embeds.hash(&mut hasher);
		message.attachments.hash(&mut hasher);
		hasher.finish()
	}
	fn new(message: &Message, text: u32, media: bool) -> Self {
		Self {
			content: Self::fingerprint(message),
			text,
			media,
		}
	}
	fn matches(&self, message: &Message) -> bool {
		self.content == Self::fingerprint(message)
	}
}
pub fn visible_range(rows: &[(Id, f32)], min: f32, max: f32) -> (usize, usize, f32) {
	let mut top = 0.0;
	let mut first = 0;
	while first < rows.len() && top + rows[first].1 <= min {
		top += rows[first].1;
		first += 1;
	}
	let mut end = first;
	let mut bottom = top;
	while end < rows.len() && bottom < max {
		bottom += rows[end].1;
		end += 1;
	}
	(first, end, top)
}
fn channel_welcome(ui: &mut egui::Ui, channel: &model::Channel, height: f32) {
	let colors = crate::design::palette(ui);
	let width = (ui.available_width() - 32.0).max(1.0);
	let heading = egui::WidgetText::from(
		crate::design::semibold(
			ui,
			crate::i18n::translate_args(
				"timeline-channel-welcome-title",
				&[("channel", &channel.name)],
			),
			28.0,
		)
		.color(colors.text_strong),
	)
	.into_galley(
		ui,
		Some(egui::TextWrapMode::Wrap),
		width,
		egui::TextStyle::Heading,
	);
	let description = egui::WidgetText::from(
		RichText::new(crate::i18n::translate(
			"timeline-channel-welcome-this-is-the-beginning-of-the-conversation",
		))
		.color(colors.muted),
	)
	.into_galley(
		ui,
		Some(egui::TextWrapMode::Wrap),
		width,
		egui::TextStyle::Body,
	);
	// Scroll contents have unbounded available height. Use the finite viewport and
	// measured, wrapped text so short channels sit above the composer at every width.
	let content_height = 32.0 + 64.0 + 16.0 + heading.size().y + 8.0 + description.size().y;
	ui.add_space((height - content_height).max(0.0));
	egui::Frame::NONE
		.inner_margin(egui::Margin::same(16))
		.show(ui, |ui| {
			let (badge, _) = ui.allocate_exact_size(egui::Vec2::splat(64.0), egui::Sense::hover());
			ui.painter()
				.circle_filled(badge.center(), 32.0, colors.raised);
			crate::icons::paint(
				ui.painter(),
				match channel.kind {
					5 => crate::icons::Icon::Megaphone,
					10..=12 => crate::icons::Icon::Thread,
					_ => crate::icons::Icon::Hash,
				},
				badge.shrink(14.0),
				colors.text_strong,
			);
			ui.add_space(16.0);
			ui.add(egui::Label::new(heading));
			ui.add_space(8.0);
			ui.add(egui::Label::new(description));
		});
}

fn loading_messages(ui: &mut egui::Ui) {
	let colors = crate::design::palette(ui);
	let height = ui.available_height();
	let (rect, response) = ui.allocate_exact_size(
		egui::vec2(ui.available_width(), height),
		egui::Sense::hover(),
	);
	response.widget_info(|| {
		egui::WidgetInfo::labeled(
			egui::Role::Label,
			false,
			crate::i18n::translate("timeline-loading-messages-loading-messages"),
		)
	});
	let painter = ui.painter().with_clip_rect(ui.clip_rect().intersect(rect));
	let fill = colors.muted.gamma_multiply(0.22);
	let text_width = (rect.width() - 88.0).clamp(0.0, 480.0);
	let rows = (height / 68.0).ceil().clamp(0.0, 128.0) as usize;
	for (index, length) in [0.85, 0.65, 0.95, 0.55]
		.into_iter()
		.cycle()
		.take(rows)
		.enumerate()
	{
		let origin = rect.min + egui::vec2(16.0, 12.0 + index as f32 * 68.0);
		painter.circle_filled(origin + egui::vec2(20.0, 20.0), 20.0, fill);
		for (y, width, height) in [
			(0.0, text_width.min(96.0), 12.0),
			(22.0, text_width * length, 10.0),
			(40.0, text_width * length * 0.7, 10.0),
		] {
			painter.rect_filled(
				egui::Rect::from_min_size(origin + egui::vec2(56.0, y), egui::vec2(width, height)),
				4,
				fill,
			);
		}
	}
}
fn ease_out_cubic(t: f32) -> f32 {
	let rest = 1.0 - t;
	1.0 - rest * rest * rest
}
fn centered_offset(rows: &[(Id, f32)], id: Id, viewport_h: f32, packed: f32) -> f32 {
	let row_top = anchor_offset(rows, id, 0.0);
	let row_h = rows
		.iter()
		.find(|(row, _)| *row == id)
		.map_or(0.0, |(_, height)| *height);
	(row_top - (viewport_h - row_h.min(viewport_h)) * 0.5)
		.clamp(0.0, (packed - viewport_h).max(0.0))
}
fn anchor_offset(rows: &[(Id, f32)], id: Id, inset: f32) -> f32 {
	if rows.is_empty() {
		return 0.0;
	}
	// Keep the next surviving message at the top; fall back to the previous one at the end.
	let index = rows
		.partition_point(|(row, _)| *row < id)
		.min(rows.len() - 1);
	let within = if rows[index].0 == id {
		// A short bottom-aligned page stores leading space as a negative inset.
		// Keep that space when older rows arrive; only the final scroll offset is nonnegative.
		inset.min(rows[index].1.max(0.0))
	} else {
		0.0
	};
	(rows[..index].iter().map(|(_, height)| *height).sum::<f32>() + within).max(0.0)
}
fn layout_key(message: &Message) -> u64 {
	// A layout fingerprint only; spoiler visibility uses exact text instead.
	let mut key = DefaultHasher::new();
	message.content.hash(&mut key);
	for user in &message.mentions {
		user.id.hash(&mut key);
		user.name.hash(&mut key);
	}
	message.author.name.hash(&mut key);
	message.author.kind.hash(&mut key);
	message.author.webhook.hash(&mut key);
	message.edited.hash(&mut key);
	message.reply_to.hash(&mut key);
	if let Some(interaction) = &message.interaction {
		interaction.user.id.hash(&mut key);
		interaction.user.name.hash(&mut key);
		interaction.command.hash(&mut key);
	}
	message.forwarded.hash(&mut key);
	message.reply_deleted.hash(&mut key);
	message.unsupported.hash(&mut key);
	message.extra_content.hash(&mut key);
	message.poll.hash(&mut key);
	message.sticker_items.hash(&mut key);
	message.components.hash(&mut key);
	message.kind.hash(&mut key);
	message.attachments.hash(&mut key);
	message.embeds.hash(&mut key);
	message.embeds_suppressed.hash(&mut key);
	match message.reactions.as_deref() {
		Some(reactions) => {
			true.hash(&mut key);
			for reaction in reactions {
				reaction.emoji.hash(&mut key);
			}
		}
		None => false.hash(&mut key),
	}
	key.finish()
}
pub(crate) const MESSAGE_LINE: f32 = 22.0;

/// Widest author name in a compact row; the body takes the rest beside it.
pub(crate) fn compact_author_width(available: f32) -> f32 {
	(available * 0.35).clamp(72.0, 150.0)
}

/// Match the first body-text baseline, including custom fonts and reading text sizes.
pub(crate) fn compact_header(
	ui: &mut egui::Ui,
	width: f32,
	font: egui::FontId,
	add: impl FnOnce(&mut egui::Ui),
) {
	let body = ui.painter().layout_no_wrap(
		" ".into(),
		egui::TextStyle::Body.resolve(ui.style()),
		egui::Color32::TRANSPARENT,
	);
	let header = ui
		.painter()
		.layout_no_wrap(" ".into(), font, egui::Color32::TRANSPARENT);
	let offset = body.rows[0].glyphs[0].pos.y - header.rows[0].glyphs[0].pos.y;
	let at = ui.next_widget_position();
	let mut header_ui = ui.new_child(
		egui::UiBuilder::new()
			.max_rect(egui::Rect::from_min_size(
				at + egui::vec2(0.0, offset),
				egui::vec2(width, body.size().y),
			))
			.layout(egui::Layout::left_to_right(egui::Align::Min)),
	);
	add(&mut header_ui);
	let used = header_ui.min_rect();
	// Negative font offsets must not move the next column's origin above this row.
	ui.advance_cursor_after_rect(egui::Rect::from_min_size(
		at,
		egui::vec2(
			used.width(),
			body.size().y.max(used.height()).max(used.bottom() - at.y),
		),
	));
}
const GROUPED_ROW_SAVINGS: f32 = 52.0;

/// Space above a new message group: cozy by default, tighter when compact spacing is on.
pub(crate) fn group_gap(compact_messages: bool) -> i8 {
	if compact_messages { 4 } else { 10 }
}

fn reserved_chrome(ui: &egui::Ui, message: &Message, width: f32) -> f32 {
	let reactions = crate::reactions::estimated_height(
		ui,
		message.reactions.as_deref(),
		(width - 88.0).max(40.0),
	);
	let components = 40.0 * (message.components.len().min(5) as f32);
	let stickers = 160.0 * (message.sticker_items.len().min(4) as f32);
	reactions
		+ components
		+ stickers
		+ message
			.poll
			.as_ref()
			.map_or(0.0, |p| 140.0 + 64.0 * p.answers.len() as f32)
}

pub(crate) fn fill_header_line(ui: &mut egui::Ui, compact: bool, text_line: egui::Rect) {
	let slack = MESSAGE_LINE - text_line.height();
	if !compact && slack > 0.0 && ui.min_rect().bottom() - text_line.bottom() < 1.0 {
		ui.expand_to_include_y(text_line.bottom() + slack);
	}
}
// Discord snowflakes carry milliseconds since 2015-01-01. All u64 IDs fit time's range.
fn timestamp(id: Id) -> time::OffsetDateTime {
	crate::local_time::local(
		time::OffsetDateTime::from_unix_timestamp(((id.0 >> 22) / 1000) as i64 + 1_420_070_400)
			.expect("snowflake timestamp is in range"),
	)
}
/// Discord's Nitro boost pink; not part of any theme palette.
const BOOST: egui::Color32 = egui::Color32::from_rgb(0xff, 0x73, 0xfa);
/// Gutter glyph and tint for a system message type, mirroring Discord's system rows.
fn system_icon(kind: u8, colors: &crate::design::Palette) -> (crate::icons::Icon, egui::Color32) {
	use crate::icons::Icon;
	match kind {
		1 | 7 => (Icon::ArrowRight, colors.positive),
		2 => (Icon::ArrowLeft, colors.danger),
		3 | 65 => (Icon::Phone, colors.positive),
		4 => (Icon::Pencil, colors.muted),
		5 => (Icon::Image, colors.muted),
		6 => (Icon::Pin, colors.muted),
		8..=11 => (Icon::Sparkle, BOOST),
		12 | 27..=31 => (Icon::Megaphone, colors.muted),
		14 | 15 => (Icon::Compass, colors.positive),
		16 | 17 => (Icon::Compass, colors.warning),
		18 | 21 => (Icon::Thread, colors.muted),
		22 => (Icon::AddPeople, colors.muted),
		24 | 36 | 38 => (Icon::ShieldWarning, colors.danger),
		37 | 39 | 62 => (Icon::ShieldWarning, colors.positive),
		58 => (Icon::Trash, colors.muted),
		59..=61 => (Icon::ShieldWarning, colors.danger),
		55 => (Icon::ScreenShare, colors.accent),
		67 => (Icon::Check, colors.positive),
		25 | 26 | 32 => (Icon::Crown, colors.warning),
		44 => (Icon::ShoppingCart, colors.accent),
		46 => (Icon::ChartBar, colors.muted),
		_ => (Icon::Help, colors.muted),
	}
}
/// "N messages · last activity" summary for a thread row, built from synced metadata only.
pub(crate) fn thread_activity(thread: &model::Channel) -> String {
	let count = match thread.message_count {
		Some(0) => crate::i18n::translate("timeline-thread-activity-no-replies"),
		Some(1) => crate::i18n::translate("timeline-thread-activity-one-message"),
		Some(n) => crate::i18n::translate_args(
			"timeline-thread-activity-many-messages",
			&[("count", &n.to_string())],
		),
		None => crate::i18n::translate("timeline-thread-activity-thread"),
	};
	let Some(last) = thread.last_message else {
		return count;
	};
	crate::i18n::translate_args(
		"timeline-thread-activity-last-active",
		&[
			("count", &count),
			("time", &crate::local_time::ago(timestamp(last))),
		],
	)
}
/// Discord-style card under a message that started a thread: name, activity, open affordance.
fn thread_card(
	ui: &mut egui::Ui,
	thread: &model::Channel,
	colors: &crate::design::Palette,
) -> egui::Response {
	let width = ui.available_width().min(520.0);
	let (rect, response) = ui.allocate_exact_size(egui::vec2(width, 56.0), egui::Sense::click());
	let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
	if ui.is_rect_visible(rect) {
		let hovered = response.hovered() || response.has_focus();
		let painter = ui.painter();
		painter.rect(
			rect,
			8.0,
			if hovered { colors.hover } else { colors.raised },
			egui::Stroke::new(1.0, colors.border),
			egui::StrokeKind::Inside,
		);
		let icon = egui::Rect::from_center_size(
			egui::pos2(rect.left() + 26.0, rect.center().y),
			egui::Vec2::splat(20.0),
		);
		crate::icons::paint(painter, crate::icons::Icon::Thread, icon, colors.muted);
		let text_left = rect.left() + 48.0;
		let text_right = rect.right() - 16.0;
		let open_width = painter
			.layout_no_wrap(
				"View thread ›".to_owned(),
				egui::FontId::proportional(13.0),
				colors.link,
			)
			.size()
			.x;
		painter.text(
			egui::pos2(text_right, rect.center().y),
			egui::Align2::RIGHT_CENTER,
			"View thread ›",
			egui::FontId::proportional(13.0),
			colors.link,
		);
		let name_width = (text_right - open_width - 12.0 - text_left).max(40.0);
		let name = egui::WidgetText::from(
			crate::design::semibold(ui, thread.name.as_str(), 14.5).color(colors.text_strong),
		)
		.into_galley(
			ui,
			Some(egui::TextWrapMode::Truncate),
			name_width,
			egui::FontSelection::Default,
		);
		painter.galley(
			egui::pos2(text_left, rect.top() + 9.0),
			name,
			colors.text_strong,
		);
		let activity = egui::WidgetText::from(
			RichText::new(thread_activity(thread))
				.size(12.5)
				.color(colors.muted),
		)
		.into_galley(
			ui,
			Some(egui::TextWrapMode::Truncate),
			name_width,
			egui::FontSelection::Default,
		);
		painter.galley(
			egui::pos2(text_left, rect.top() + 30.0),
			activity,
			colors.muted,
		);
	}
	response.on_hover_text(crate::i18n::translate_args(
		"timeline-open-thread",
		&[("thread", &thread.name)],
	))
}
/// The message a thread hangs off, shown above its replies like Discord's thread view.
fn starter_row(
	ui: &mut egui::Ui,
	message: &Message,
	state: &State,
	avatars: &mut crate::avatars::Avatars,
	width: f32,
) {
	let colors = crate::design::palette(ui);
	egui::Frame::NONE
		.inner_margin(egui::Margin {
			left: 16,
			right: 16,
			top: group_gap(false),
			bottom: 6,
		})
		.show(ui, |ui| {
			ui.set_min_width((width - 32.0).max(1.0));
			ui.spacing_mut().item_spacing = egui::vec2(16.0, 4.0);
			ui.horizontal_top(|ui| {
				avatars.show_plain(ui, &message.author, 40.0, state.demo);
				ui.vertical(|ui| {
					ui.set_width(ui.available_width());
					ui.allocate_ui_with_layout(
						egui::vec2(ui.available_width(), 22.0),
						egui::Layout::left_to_right(egui::Align::Center),
						|ui| {
							ui.spacing_mut().item_spacing.x = 8.0;
							crate::account_badge::name(
								ui,
								&message.author,
								state.message_author_name(message),
								15.5,
								(
									colors.text_strong,
									state.message_author_colors(message),
									colors.chat,
								),
								egui::Sense::hover(),
								0.0,
							);
							let time = timestamp(message.id);
							ui.label(
								RichText::new(format!("{:02}:{:02}", time.hour(), time.minute()))
									.size(12.0)
									.color(colors.muted),
							)
							.on_hover_text_with(|| format!("{time} UTC"));
						},
					);
					ui.add(
						egui::Label::new(RichText::new(message.display_text()).color(colors.text))
							.wrap()
							.selectable(false),
					);
				});
			});
			ui.add_space(10.0);
			// A labelled rule separates the starter from the replies that followed it.
			ui.horizontal(|ui| {
				ui.spacing_mut().item_spacing.x = 8.0;
				let (line, _) = ui.allocate_exact_size(
					egui::vec2((ui.available_width() - 220.0).max(24.0), 1.0),
					egui::Sense::hover(),
				);
				ui.painter().hline(
					line.x_range(),
					line.center().y,
					egui::Stroke::new(1.0, colors.border),
				);
				crate::icons::inline(ui, crate::icons::Icon::Thread, 14.0, colors.muted);
				ui.label(
					RichText::new(crate::i18n::translate(
						"timeline-starter-row-thread-started-from-this-message",
					))
					.size(12.0)
					.color(colors.muted),
				);
				let (line, _) = ui.allocate_exact_size(
					egui::vec2(ui.available_width(), 1.0),
					egui::Sense::hover(),
				);
				ui.painter().hline(
					line.x_range(),
					line.center().y,
					egui::Stroke::new(1.0, colors.border),
				);
			});
		});
}
/// Loaded and private (ephemeral) messages of the selected conversation in id order.
fn display_rows<'a>(state: &'a State) -> impl Iterator<Item = &'a Message> + 'a {
	let mut private: Vec<&Message> = state
		.interactions
		.ephemeral
		.iter()
		.filter(|message| Some(message.channel) == state.selected)
		.collect();
	private.sort_by_key(|message| message.id);
	let mut shared = state.timeline.display_iter().peekable();
	let mut private = private.into_iter().peekable();
	std::iter::from_fn(move || match (shared.peek(), private.peek()) {
		(Some(shared_next), Some(private_next)) if private_next.id < shared_next.id => {
			private.next()
		}
		(Some(_), _) => shared.next(),
		(None, _) => private.next(),
	})
}
/// A row payload, including retained deleted bodies and private command responses.
fn display_message(state: &State, id: Id) -> Option<&Message> {
	state.timeline.get_display(id).or_else(|| {
		state
			.interactions
			.ephemeral
			.iter()
			.find(|message| message.id == id && Some(message.channel) == state.selected)
	})
}
fn queue_missing_channel_reference(
	queued: &mut Option<Id>,
	formatted: &Formatted,
	state: &State,
	revealed: u32,
) {
	if queued.is_none() {
		*queued = formatted.missing_channel_reference(state, revealed);
	}
}
/// The curved gutter connector shared by reply and command-invocation headers.
fn reference_spine(ui: &mut egui::Ui, colors: &crate::design::Palette) {
	let (gutter, _) = ui.allocate_exact_size(egui::vec2(50.0, 18.0), egui::Sense::hover());
	let x = gutter.left() + 20.0;
	let y = gutter.center().y;
	let stroke = egui::Stroke::new(2.0, colors.muted.gamma_multiply(0.5));
	ui.painter().line_segment(
		[egui::pos2(x, gutter.bottom() + 2.0), egui::pos2(x, y + 5.0)],
		stroke,
	);
	ui.painter()
		.add(egui::epaint::QuadraticBezierShape::from_points_stroke(
			[
				egui::pos2(x, y + 5.0),
				egui::pos2(x, y),
				egui::pos2(x + 5.0, y),
			],
			false,
			egui::Color32::TRANSPARENT,
			stroke,
		));
	ui.painter().line_segment(
		[egui::pos2(x + 5.0, y), egui::pos2(gutter.right(), y)],
		stroke,
	);
}
fn grouped(previous: Option<&Message>, message: &Message, boundary: Option<Id>) -> bool {
	previous.is_some_and(|previous| {
		previous.author.id == message.author.id
			&& previous.author.account_label() == message.author.account_label()
			&& (previous.author.kind == model::AccountKind::VerifiedBot)
				== (message.author.kind == model::AccountKind::VerifiedBot)
			&& message.reply_to.is_none()
			&& message.interaction.is_none()
			&& !message.ephemeral
			&& !previous.ephemeral
			&& !message.unsupported
			&& !previous.unsupported
			&& !message.extra_content.any()
			&& !previous.extra_content.any()
			&& boundary != Some(message.id)
			&& timestamp(previous.id).date() == timestamp(message.id).date()
			&& (timestamp(message.id) - timestamp(previous.id)).whole_seconds() < 300
	})
}

pub(crate) fn mentions_viewer(message: &Message, state: &State) -> bool {
	let viewer = state.user.as_ref().map(|user| user.id);
	message.mention_everyone
		|| viewer.is_some_and(|viewer| message.mentions.iter().any(|mention| mention.id == viewer))
		|| (viewer.is_some()
			&& state
				.channel(message.channel)
				.and_then(|channel| channel.guild)
				.and_then(|guild| state.permissions.guilds.get(&guild))
				.and_then(|guild| guild.member.as_ref())
				.is_some_and(|member| {
					message
						.mention_roles
						.iter()
						.any(|role| member.roles.contains(role))
				}))
}
fn row_key(
	message: &Message,
	previous: Option<&Message>,
	boundary: Option<Id>,
	state: &State,
) -> u64 {
	let mut key = DefaultHasher::new();
	layout_key(message).hash(&mut key);
	grouped(previous, message, boundary).hash(&mut key);
	previous
		.is_none_or(|previous| timestamp(previous.id).date() != timestamp(message.id).date())
		.hash(&mut key);
	(boundary == Some(message.id)).hash(&mut key);
	crate::mentions::presentation_fingerprint(state, message).hash(&mut key);
	key.finish()
}
fn row_height_key(
	message: &Message,
	previous: Option<&Message>,
	boundary: Option<Id>,
	state: &State,
	deleted: bool,
) -> u64 {
	row_key(message, previous, boundary, state) ^ u64::from(deleted)
}
fn divider(ui: &mut egui::Ui, label: String, unread: bool, compact: bool) {
	let colors = crate::design::palette(ui);
	let color = if unread { colors.danger } else { colors.muted };
	ui.add_space(if compact { 8.0 } else { 16.0 });
	ui.horizontal(|ui| {
		ui.add_space(16.0);
		let font = egui::FontId::new(12.0, crate::design::semibold_family(ui.ctx()));
		let text = ui.painter().layout_no_wrap(label.clone(), font, color);
		let (rect, response) = ui.allocate_exact_size(
			egui::vec2((ui.available_width() - 16.0).max(0.0), 20.0),
			crate::select::band_sense(),
		);
		response
			.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Label, ui.is_enabled(), &label));
		let pos = egui::pos2(
			rect.center().x - text.size().x / 2.0,
			rect.center().y - text.size().y / 2.0,
		);
		let gap = (rect.width() - text.size().x - 24.0).max(0.0) / 2.0;
		for (a, b) in [
			(rect.left(), rect.left() + gap),
			(rect.right() - gap, rect.right()),
		] {
			ui.painter().line_segment(
				[
					egui::pos2(a, rect.center().y),
					egui::pos2(b, rect.center().y),
				],
				egui::Stroke::new(1.0, if unread { color } else { colors.border }),
			);
		}
		egui::text_selection::LabelSelectionState::label_text_selection(
			ui,
			&response,
			pos,
			text,
			color,
			egui::Stroke::NONE,
		);
		if response.hovered() {
			ui.ctx().set_cursor_icon(egui::CursorIcon::Default);
		}
	});
	ui.add_space(if compact { 2.0 } else { 4.0 });
}
/// A hover-bar reaction: its emoji, whether the reader already reacted, and whether it toggles now.
fn quick_reaction(
	state: &State,
	message: &Message,
	text: &str,
) -> (model::ReactionEmoji, bool, bool) {
	let emoji = model::ReactionEmoji {
		id: None,
		name: Some(text.to_owned()),
	};
	let reactions = state.reactions.display(message);
	let reacted = reactions
		.and_then(|items| items.iter().find(|r| r.emoji.same(&emoji)))
		.is_some_and(|r| r.me);
	let enabled = reactions.is_some()
		&& !state.reactions.busy()
		&& state.can_react(message.id, Some(&emoji), !reacted);
	(emoji, reacted, enabled)
}
fn action_button(ui: &mut egui::Ui, icon: crate::icons::Icon, label: &str) -> egui::Response {
	crate::icons::button_immediate(ui, icon, 28.0, label)
}

/// One transient confirmation for all direct copy actions; never retains copied text.
fn copy_action_button(
	ui: &mut egui::Ui,
	icon: crate::icons::Icon,
	label: &str,
	key: egui::Id,
	feedback: &mut Option<(egui::Id, f64)>,
	text: impl FnOnce() -> String,
) -> egui::Response {
	let now = ui.input(|input| input.time);
	if feedback.is_some_and(|(_, at)| now - at >= 1.5) {
		*feedback = None;
	}
	let age = feedback
		.filter(|(id, _)| *id == key)
		.map(|(_, at)| now - at);
	let (rect, response) = ui.allocate_exact_size(egui::Vec2::splat(28.0), egui::Sense::click());
	let clicked = response.clicked();
	if clicked {
		ui.ctx().copy_text(text());
		*feedback = Some((key, now));
	}
	let age = if clicked { Some(0.0) } else { age };
	let colors = crate::design::palette(ui);
	if response.hovered() || response.has_focus() {
		ui.painter().rect_filled(rect, 6, colors.hover);
	}
	let label = crate::i18n::translate(if age.is_some() {
		"updates-update-settings-copied"
	} else {
		label
	});
	let (icon, scale, color) = if let Some(age) = age {
		let t = (age as f32 / 0.2).clamp(0.0, 1.0);
		let scale =
			0.65 + 0.35 * (1.0 - (1.0 - t).powi(3)) + 0.12 * (std::f32::consts::PI * t).sin();
		if age < 0.2 {
			ui.ctx().request_repaint();
		} else {
			ui.ctx()
				.request_repaint_after(std::time::Duration::from_secs_f64(1.5 - age));
		}
		(crate::icons::Icon::Check, scale, colors.positive)
	} else {
		(
			icon,
			1.0,
			if response.hovered() || response.has_focus() {
				colors.text_strong
			} else {
				colors.muted
			},
		)
	};
	crate::icons::paint(
		ui.painter(),
		icon,
		egui::Rect::from_center_size(rect.center(), egui::Vec2::splat(16.8 * scale)),
		color,
	);
	response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &label));
	crate::icons::immediate_tooltip(response, &label)
}

#[cfg(all(debug_assertions, feature = "demo"))]
pub fn debug_copy_feedback_check() {
	let ctx = egui::Context::default();
	crate::design::apply(&ctx);
	let key = egui::Id::unique("debug-copy");
	let mut feedback = None;
	let mut frame = |time, events| {
		ctx.run_ui(
			egui::RawInput {
				time: Some(time),
				events,
				..Default::default()
			},
			|ui| {
				copy_action_button(
					ui,
					crate::icons::Icon::Copy,
					"message-menu-copy",
					key,
					&mut feedback,
					|| "Synthetic copied message".into(),
				);
			},
		)
	};
	let initial = frame(0.0, vec![]);
	let point = egui::pos2(14.0, 14.0);
	frame(
		0.1,
		vec![
			egui::Event::PointerMoved(point),
			egui::Event::PointerButton {
				pos: point,
				button: egui::PointerButton::Primary,
				pressed: true,
				modifiers: egui::Modifiers::SHIFT,
			},
		],
	)
	.drop_without_applying_deltas();
	let copied = frame(
		0.2,
		vec![egui::Event::PointerButton {
			pos: point,
			button: egui::PointerButton::Primary,
			pressed: false,
			modifiers: egui::Modifiers::SHIFT,
		}],
	);
	assert!(copied.platform_output.commands.iter().any(|command|
		matches!(command, egui::OutputCommand::CopyText(text) if text == "Synthetic copied message")));
	let settled = frame(0.5, vec![]);
	let expired = frame(1.8, vec![]);
	let icon = |output: &egui::FullOutput| {
		output
			.shapes
			.iter()
			.find_map(|shape| match &shape.shape {
				egui::Shape::Mesh(mesh) => Some((mesh.vertices[0].uv, mesh.calc_bounds().width())),
				_ => None,
			})
			.expect("copy action icon")
	};
	assert_ne!(
		icon(&initial).0,
		icon(&copied).0,
		"copy changes to a checkmark"
	);
	assert!(
		icon(&copied).1 < icon(&settled).1,
		"checkmark grows into place"
	);
	assert_eq!(
		icon(&initial).0,
		icon(&expired).0,
		"copy icon returns after confirmation"
	);
	assert!(settled.platform_output.commands.is_empty());
	assert!(expired.platform_output.commands.is_empty());
	for output in [initial, copied, settled, expired] {
		output.drop_without_applying_deltas();
	}
	println!("Copy action clipboard, animated checkmark and expiry checks passed.");
}

enum DeletedLocalAction {
	ToggleHighlight,
	Remove,
}

fn deleted_message_actions(popup: egui::Popup<'_>, action: &mut Option<DeletedLocalAction>) {
	popup.show(|ui| {
		ui.set_min_width(160.0);
		if ui
			.button(crate::i18n::translate(
				"timeline-deleted-message-actions-toggle-deleted-highlight",
			))
			.clicked()
		{
			*action = Some(DeletedLocalAction::ToggleHighlight);
			ui.close();
		}
		if ui
			.button(crate::i18n::translate(
				"timeline-deleted-message-actions-remove-message",
			))
			.clicked()
		{
			*action = Some(DeletedLocalAction::Remove);
			ui.close();
		}
	});
}
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn message_actions(
	popup: egui::Popup<'_>,
	(message, link, extension_actions, extension_request): (
		&Message,
		Option<&str>,
		&[crate::extensions_ui::MenuAction],
		&mut Option<(crate::extensions_ui::MenuAction, String)>,
	),
	actions: (bool, bool, bool, bool),
	selection: (
		Option<&mut Option<Id>>,
		Option<&mut Option<Id>>,
		&mut Option<Id>,
	),
	editing: (&mut Option<(Id, Id, String)>, &mut bool),
	deleting: &mut Option<(Id, Id)>,
	pin: (bool, bool, &mut Option<(Id, Id, bool)>),
	thread: (bool, &mut Option<(Id, Id)>),
	forward: (bool, &mut Option<Id>),
	view_reactions: Option<(
		model::ReactionEmoji,
		&mut Option<(Id, model::ReactionEmoji, bool)>,
	)>,
) {
	let (mark_read, mark_unread, reply) = selection;
	let (editing, edit_started) = editing;
	let (own, can_reply, can_edit, can_delete) = actions;
	let (can_pin, pinned, pin_request) = pin;
	let (can_thread, thread_request) = thread;
	popup.show(|ui| {
		ui.set_min_width(160.0);
		if !extension_actions.is_empty() {
			ui.menu_button(
				crate::i18n::translate("timeline-message-actions-extensions"),
				|ui| {
					for action in extension_actions {
						if ui.button(&action.label).clicked() {
							*extension_request =
								Some((action.clone(), message.display_text().into_owned()));
							ui.close();
						}
					}
				},
			);
			ui.separator();
		}
		if crate::select::has_selection(ui.ctx())
			&& ui
				.button(crate::i18n::translate("timeline-message-actions-copy"))
				.clicked()
		{
			crate::select::request_copy(ui.ctx());
			ui.close();
		}
		if ui
			.button(crate::i18n::translate("message-menu-copy"))
			.clicked()
		{
			ui.ctx().copy_text(message.display_text().into_owned());
			ui.close();
		}
		if let Some(link) = link
			&& ui
				.button(crate::i18n::translate(
					"timeline-message-actions-copy-message-link",
				))
				.clicked()
		{
			ui.ctx().copy_text(link.to_owned());
			ui.close();
		}
		if ui
			.button(crate::i18n::translate(
				"timeline-message-actions-copy-message-id",
			))
			.clicked()
		{
			ui.ctx().copy_text(message.id.to_string());
			ui.close();
		}
		if ui
			.add_enabled(
				can_reply,
				egui::Button::new(crate::i18n::translate("timeline-message-actions-reply")),
			)
			.clicked()
		{
			*reply = Some(message.id);
			ui.close();
		}
		if ui
			.add_enabled(
				forward.0,
				egui::Button::new(crate::i18n::translate("timeline-message-actions-forward")),
			)
			.clicked()
		{
			*forward.1 = Some(message.id);
			ui.close();
		}
		if can_thread
			&& ui
				.button(crate::i18n::translate(
					"timeline-message-actions-create-thread",
				))
				.clicked()
		{
			*thread_request = Some((message.channel, message.id));
			ui.close();
		}
		if let Some((emoji, view)) = view_reactions
			&& ui
				.button(crate::i18n::translate(
					"timeline-message-actions-view-reactions",
				))
				.clicked()
		{
			*view = Some((message.id, emoji, true));
			ui.close();
		}
		if ui
			.add_enabled(
				mark_read.is_some(),
				egui::Button::new(crate::i18n::translate(
					"timeline-message-actions-mark-read-through-here",
				)),
			)
			.clicked()
		{
			if let Some(mark_read) = mark_read {
				*mark_read = Some(message.id);
			}
			ui.close();
		}
		if ui
			.add_enabled(
				mark_unread.is_some(),
				egui::Button::new(crate::i18n::translate(
					"timeline-message-actions-mark-unread",
				)),
			)
			.clicked()
		{
			if let Some(mark_unread) = mark_unread {
				*mark_unread = Some(message.id);
			}
			ui.close();
		}
		if ui
			.add_enabled(
				can_pin,
				egui::Button::new(crate::i18n::translate_if_key(if pinned {
					"timeline-message-actions-unpin-message"
				} else {
					"timeline-message-actions-pin-message"
				})),
			)
			.clicked()
		{
			*pin_request = Some((message.channel, message.id, !pinned));
			ui.close();
		}
		if own || can_delete {
			ui.separator();
		}
		if own
			&& ui
				.add_enabled(
					can_edit,
					egui::Button::new(crate::i18n::translate(
						"timeline-message-actions-edit-message",
					)),
				)
				.clicked()
		{
			*editing = Some((message.channel, message.id, message.content.clone()));
			*edit_started = true;
			ui.close();
		}
		if (own || can_delete)
			&& ui
				.add_enabled(
					can_delete,
					egui::Button::new(crate::i18n::translate(
						"timeline-message-actions-delete-message",
					)),
				)
				.clicked()
		{
			*deleting = Some((message.channel, message.id));
			ui.close();
		}
	});
}
/// Flat strip painted over the timeline edge; `add` lays out its contents left to right.
fn overlay_bar(
	ui: &mut egui::Ui,
	rect: egui::Rect,
	fill: egui::Color32,
	radius: egui::CornerRadius,
	add: impl FnOnce(&mut egui::Ui),
) {
	if radius.sw == 0 {
		// Bottom bars cast a soft shadow upward onto the messages behind them.
		ui.painter().rect_filled(
			rect.expand2(egui::vec2(1.0, 0.0))
				.translate(egui::vec2(0.0, -1.0)),
			egui::CornerRadius {
				nw: 9,
				ne: 9,
				sw: 0,
				se: 0,
			},
			egui::Color32::from_black_alpha(48),
		);
	}
	ui.painter().rect_filled(rect, radius, fill);
	let mut bar = ui.new_child(
		egui::UiBuilder::new()
			.scope_id(ui.make_persistent_id(("timeline-overlay", radius.sw == 0)))
			.max_rect(rect.shrink2(egui::vec2(12.0, 0.0)))
			.layout(egui::Layout::left_to_right(egui::Align::Center)),
	);
	bar.spacing_mut().item_spacing.x = 8.0;
	add(&mut bar);
}
/// "Back to the live edge" control floating over the bottom-right of the conversation: a round
/// arrow, or an accent pill naming the messages that arrived below while the reader was away.
fn present_control(
	ui: &mut egui::Ui,
	area: egui::Rect,
	unread: bool,
	new_below: Option<&str>,
) -> (egui::Rect, bool) {
	let colors = crate::design::palette(ui);
	let size = 38.0;
	let galley = new_below.map(|label| {
		egui::WidgetText::from(crate::design::medium(ui, label, 14.0).color(colors.accent_text))
			.into_galley(
				ui,
				Some(egui::TextWrapMode::Truncate),
				(area.width() - 32.0 - size).max(size),
				egui::TextStyle::Body,
			)
	});
	let width = galley
		.as_ref()
		.map_or(size, |galley| 16.0 + galley.size().x + 6.0 + 18.0 + 12.0);
	let rect = egui::Rect::from_min_size(
		egui::pos2(
			area.right() - 16.0 - width,
			area.bottom() - crate::typing::OVERLAY_HEIGHT - 10.0 - size,
		),
		egui::vec2(width, size),
	);
	let response = ui.interact(
		rect,
		ui.make_persistent_id("timeline-present"),
		egui::Sense::click(),
	);
	let painter = ui.painter();
	let radius = egui::CornerRadius::same((size / 2.0) as u8);
	painter.rect_filled(
		rect.translate(egui::vec2(0.0, 1.5)),
		radius,
		egui::Color32::from_black_alpha(52),
	);
	let accent = unread || galley.is_some();
	let fill = if accent {
		colors.accent
	} else {
		colors.raised.to_opaque()
	};
	let fill = if response.hovered() {
		fill.gamma_multiply(1.12)
	} else {
		fill
	};
	painter.rect_filled(rect, radius, fill);
	if !accent {
		painter.rect_stroke(
			rect,
			radius,
			egui::Stroke::new(1.0, colors.border),
			egui::StrokeKind::Inside,
		);
	}
	let color = if accent {
		colors.accent_text
	} else {
		colors.text_strong
	};
	let arrow_center = if let Some(galley) = &galley {
		let text_pos = egui::pos2(rect.left() + 16.0, rect.center().y - galley.size().y / 2.0);
		painter.galley(text_pos, galley.clone(), color);
		egui::pos2(rect.right() - 12.0 - 9.0, rect.center().y)
	} else {
		rect.center()
	};
	crate::icons::paint(
		painter,
		crate::icons::Icon::ArrowDown,
		egui::Rect::from_center_size(arrow_center, egui::Vec2::splat(18.0)),
		color,
	);
	if response.has_focus() {
		painter.rect_stroke(
			rect.expand(2.0),
			egui::CornerRadius::same((size / 2.0 + 2.0) as u8),
			egui::Stroke::new(1.0, colors.accent),
			egui::StrokeKind::Outside,
		);
	}
	let name = crate::i18n::translate("timeline-present-control-jump-to-present");
	let name = new_below.map_or(name.clone(), |label| format!("{label} · {name}"));
	response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &name));
	let clicked = response
		.on_hover_text(if unread || new_below.is_some() {
			crate::i18n::translate("timeline-present-control-new-messages-below-jump-to-present")
		} else {
			crate::i18n::translate("timeline-present-control-jump-to-present")
		})
		.clicked();
	(rect, clicked)
}
/// Frameless text action with a trailing arrow glyph, for use inside [`overlay_bar`].
fn bar_button(
	ui: &mut egui::Ui,
	label: &str,
	icon: crate::icons::Icon,
	color: egui::Color32,
) -> egui::Response {
	// Right-to-left layouts place the first item at the right edge, so the glyph goes first.
	let rtl = ui.layout().horizontal_placement() == egui::Align::Max;
	let glyph = |ui: &mut egui::Ui| {
		crate::icons::inline(ui, icon, 14.0, color);
	};
	if rtl {
		glyph(ui);
	}
	ui.spacing_mut().item_spacing.x = 4.0;
	let response = ui.add(
		egui::Button::new(crate::design::medium(ui, label, 13.0).color(color))
			.frame(false)
			.small(),
	);
	if !rtl {
		glyph(ui);
	}
	ui.spacing_mut().item_spacing.x = 12.0;
	response
}

fn banner_rect(area: egui::Rect) -> egui::Rect {
	egui::Rect::from_min_size(
		egui::pos2(area.left() + 16.0, area.top()),
		egui::vec2((area.width() - 32.0).max(120.0), 28.0),
	)
}

/// Returns whether the reader asked to jump to unread or mark the channel read.
/// Discord's unread bar; `summary` is "N new messages since HH:MM" once the divider is loaded.
fn unread_banner(
	ui: &mut egui::Ui,
	rect: egui::Rect,
	jump: bool,
	summary: Option<&str>,
) -> (bool, bool) {
	let colors = crate::design::palette(ui);
	let mut jump_unread = false;
	let mut mark_read = false;
	let text = colors.accent_text;
	overlay_bar(
		ui,
		rect,
		colors.accent,
		egui::CornerRadius {
			nw: 0,
			ne: 0,
			sw: 8,
			se: 8,
		},
		|ui| {
			// Buttons first, so a long summary truncates into the space they leave.
			ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
				let mark = crate::i18n::translate("timeline-unread-banner-mark-as-read");
				if bar_button(ui, &mark, crate::icons::Icon::Check, text).clicked() {
					mark_read = true;
				}
				let jump_label = crate::i18n::translate("timeline-unread-banner-jump-to-unread");
				if jump && bar_button(ui, &jump_label, crate::icons::Icon::ArrowUp, text).clicked()
				{
					jump_unread = true;
				}
				ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
					ui.add(
						egui::Label::new(
							crate::design::medium(
								ui,
								summary.map_or_else(
									|| {
										crate::i18n::translate(
											"timeline-unread-banner-unread-messages",
										)
									},
									str::to_owned,
								),
								13.0,
							)
							.color(text),
						)
						.truncate(),
					);
				});
			});
		},
	);
	(jump_unread, mark_read)
}
/// Discord-style system row: muted sentence, strong clickable names, inline timestamp.
#[allow(clippy::too_many_arguments)]
fn show_system(
	ui: &mut egui::Ui,
	system: &model::SystemMessage,
	time: time::OffsetDateTime,
	state: &State,
	profile: &mut crate::profiles::ProfileSession,
	user_action: &mut Option<crate::user_menu::Action>,
	surface: &mut crate::select::Surface,
	// A "started a thread" row: the known thread, its channel, and where clicks go.
	thread: Option<(Option<Id>, Id)>,
	targets: (&mut Option<Id>, &mut Option<Id>),
) {
	let (open_thread, open_all) = targets;
	{
		let colors = crate::design::palette(ui);
		ui.horizontal_wrapped(|ui| {
			ui.spacing_mut().item_spacing = egui::vec2(0.0, 2.0);
			for segment in &system.segments {
				if !segment.strong {
					let (pos, galley, response) =
						egui::Label::new(RichText::new(&segment.text).color(colors.muted))
							.wrap()
							.selectable(false)
							.layout_in_ui(ui);
					surface.run(ui, &response, pos, galley, Vec::new());
					continue;
				}
				let text = crate::design::medium(ui, &segment.text, 15.0).color(colors.text_strong);
				let Some(user) = &segment.user else {
					// The thread name in a "started a thread" row opens the thread itself.
					if let Some((Some(id), _)) = thread {
						let response = ui
							.add(egui::Label::new(text).sense(egui::Sense::click()))
							.on_hover_cursor(egui::CursorIcon::PointingHand)
							.on_hover_text(crate::i18n::translate_args(
								"timeline-open-thread",
								&[("thread", &segment.text)],
							));
						surface.keep(&response);
						if response.clicked() {
							*open_thread = Some(id);
						}
						continue;
					}
					let (pos, galley, response) = egui::Label::new(text)
						.wrap()
						.selectable(false)
						.layout_in_ui(ui);
					surface.run(ui, &response, pos, galley, Vec::new());
					continue;
				};
				let response = ui
					.add(egui::Label::new(text).sense(egui::Sense::click()))
					.on_hover_cursor(egui::CursorIcon::PointingHand);
				surface.keep(&response);
				crate::user_menu::show(&response, state, user, profile, user_action);
				profile.person_click(ui, &response, None, user);
			}
			if let Some((_, parent)) = thread {
				let (pos, galley, response) = egui::Label::new(
					RichText::new(crate::i18n::translate("timeline-show-system-see-all"))
						.color(colors.muted),
				)
				.wrap()
				.selectable(false)
				.layout_in_ui(ui);
				surface.run(ui, &response, pos, galley, Vec::new());
				ui.add_space(4.0);
				let all = ui
					.add(
						egui::Label::new(
							crate::design::medium(
								ui,
								crate::i18n::translate("timeline-threads"),
								15.0,
							)
							.color(colors.text_strong),
						)
						.sense(egui::Sense::click()),
					)
					.on_hover_cursor(egui::CursorIcon::PointingHand)
					.on_hover_text(crate::i18n::translate(
						"timeline-show-system-open-this-channels-threads",
					));
				surface.keep(&all);
				if all.clicked() {
					*open_all = Some(parent);
				}
				let (pos, galley, response) =
					egui::Label::new(RichText::new(".").color(colors.muted))
						.wrap()
						.selectable(false)
						.layout_in_ui(ui);
				surface.run(ui, &response, pos, galley, Vec::new());
			}
			ui.add_space(8.0);
			let stamp = ui
				.label(
					RichText::new(format!("{:02}:{:02}", time.hour(), time.minute()))
						.size(12.0)
						.color(colors.muted),
				)
				.on_hover_text_with(|| format!("{time} UTC"));
			surface.exclude(stamp.rect);
		});
	}
}
impl TimelineView {
	pub(super) fn show_fullscreen_video(&mut self, ctx: &egui::Context, state: &State) -> bool {
		if self.video.is_fullscreen() {
			let current = self
				.video
				.active
				.as_ref()
				.and_then(|(channel, id, attachment)| {
					state
						.timeline
						.get(*id)
						.or_else(|| {
							state
								.interactions
								.ephemeral
								.iter()
								.find(|message| message.id == *id)
						})
						.filter(|message| {
							state.selected == Some(*channel)
								&& message.channel == *channel
								&& message.attachments.contains(attachment)
								&& (self.component_viewing
									== Some((
										message.id,
										egui::Id::unique((
											&message.components,
											&message.attachments,
										))
										.value(),
									)) || !crate::embeds::has_media_spoilers(message)
									|| self.revealed.get(id).is_some_and(|reveal| {
										reveal.media && reveal.matches(message)
									}))
						})
						.map(|message| (message, attachment.clone()))
				});
			if let Some((message, attachment)) = current {
				self.video.show_fullscreen(
					ctx,
					message,
					&attachment,
					&mut self.download,
					&mut self.opening,
					state.demo,
				);
				return true;
			} else {
				self.video.stop();
			}
		}
		false
	}

	/// Fixture-only: open the media viewer on one attachment.
	#[cfg(any(test, feature = "demo"))]
	pub(super) fn preview_image_viewer(&mut self, message: Id, attachment: Id) {
		self.pending_viewer = Some((message, attachment));
	}
	pub(super) fn viewing_latest(&self, channel: Id) -> bool {
		self.channel == Some(channel) && self.following && self.at_current_latest
	}
	/// An explicit "Mark unread" starts a new section at the chosen message.
	pub(super) fn reset_unread_divider(&mut self) {
		self.unread_boundary = None;
		self.unread_session = false;
		self.unread_dismissed = None;
		self.revision = u64::MAX;
	}
	/// Newest message the reader has already read during this visit, when it lies inside the
	/// divider's section: the banner counts only what arrived after it.
	fn read_through(&self) -> Option<Id> {
		self.unread_dismissed.flatten().filter(|read| {
			self.unread_boundary
				.is_some_and(|boundary| boundary <= *read)
		})
	}
	/// "12 new messages since 14:05" for the divider's section; "50+" when older unread
	/// messages are not loaded yet. Messages read during the visit are not counted again.
	fn unread_summary(&self, state: &State) -> Option<String> {
		let boundary = self.unread_boundary?;
		let read_through = self.read_through();
		let start = match read_through {
			Some(read) => state.timeline.iter().find(|m| m.id > read)?.id,
			None => boundary,
		};
		let count = state.timeline.iter().filter(|m| m.id >= start).count();
		let first_loaded = state.timeline.iter().next().map(|m| m.id) == Some(boundary);
		let marker_loaded = state
			.selected
			.and_then(|channel| state.read_marker(channel))
			.flatten()
			.is_some_and(|read| state.timeline.get(read).is_some());
		let more =
			read_through.is_none() && first_loaded && !marker_loaded && !state.older_exhausted;
		let at = timestamp(start);
		let clock = format!("{:02}:{:02}", at.hour(), at.minute());
		let time = if at.date() == crate::local_time::now().date() {
			clock
		} else {
			format!("{} {}, {clock}", at.month(), at.day())
		};
		Some(crate::i18n::translate_count(
			if more {
				"timeline-unread-banner-new-since-more"
			} else {
				"timeline-unread-banner-new-since"
			},
			count,
			&[("time", &time)],
		))
	}
	/// Leaving the latest page is deliberate reading; nothing is acknowledged automatically.
	pub(super) fn browse_away(&mut self) {
		self.target_browsing = true;
		self.following = false;
		self.jump = false;
		self.reveal_scroll = None;
		self.present_scroll = None;
		self.pending_reveal = None;
		self.auto_read_attempt = None;
		self.mark_read = None;
		self.mark_unread = None;
		self.seen_latest = None;
	}
	pub(super) fn follow_latest(&mut self, state: &State) {
		self.latest |= state.history_targeted
			|| state.history_before.is_some()
			|| state.history_after.is_some();
		self.target_browsing = false;
		self.hold_read_ack = false;
		self.auto_read_attempt = None;
		self.following = true;
		self.jump = true;
		self.anchor = None;
		self.reveal_scroll = None;
		self.present_scroll = None;
		self.pending_reveal = None;
	}
	pub(super) fn request_reply_target(&mut self, id: Id) {
		self.present_scroll = None;
		self.reply_target = Some(id);
		self.pending_reveal = Some(if self.following && self.at_current_latest {
			TargetReveal::StayIfVisible
		} else {
			TargetReveal::Center
		});
	}
	#[cfg(test)]
	pub fn show(
		&mut self,
		ui: &mut egui::Ui,
		state: &mut State,
		editing: &mut Option<(Id, Id, String)>,
		deleting: &mut Option<(Id, Id)>,
		(avatars, profile): (
			&mut crate::avatars::Avatars,
			&mut crate::profiles::ProfileSession,
		),
		upload: Option<&crate::pending::Upload>,
	) {
		let mut scroll = crate::scroll::Session::default();
		self.show_with_scroll(
			ui,
			state,
			editing,
			deleting,
			(avatars, profile),
			upload,
			&mut scroll,
		);
	}

	#[allow(clippy::too_many_arguments)]
	pub fn show_with_scroll(
		&mut self,
		ui: &mut egui::Ui,
		state: &mut State,
		editing: &mut Option<(Id, Id, String)>,
		deleting: &mut Option<(Id, Id)>,
		(avatars, profile): (
			&mut crate::avatars::Avatars,
			&mut crate::profiles::ProfileSession,
		),
		upload: Option<&crate::pending::Upload>,
		session: &mut crate::scroll::Session,
	) {
		let width = ui.available_width();
		let channel_changed = self.channel != state.selected;
		if channel_changed {
			if let Some(previous) = self.channel {
				let cursor = if self.following {
					client_core::ReadingCursor {
						message: None,
						inset: 0.0,
					}
				} else {
					client_core::ReadingCursor {
						message: self.anchor.map(|(id, _)| id),
						inset: self.anchor.map(|(_, inset)| inset).unwrap_or(0.0),
					}
				};
				state.remember_reading(previous, cursor);
			}
			let restored = state.selected.and_then(|id| state.reading(id));
			let following = restored.is_none_or(|cursor| cursor.message.is_none());
			*self = Self {
				extension_actions: self.extension_actions.clone(),
				quick_reactions: self.quick_reactions,
				double_click_reaction: self.double_click_reaction,
				compact_messages: self.compact_messages,
				hide_media_links: self.hide_media_links,
				instant_scrolling: self.instant_scrolling,
				suppressed_deleted_highlight: std::mem::take(
					&mut self.suppressed_deleted_highlight,
				),
				channel: state.selected,
				following,
				anchor: restored.and_then(|cursor| cursor.message.map(|id| (id, cursor.inset))),
				download: std::mem::take(&mut self.download),
				opening: self.opening.take(),
				browser_opening: self.browser_opening.take(),
				pending_viewer: self.pending_viewer.take(),
				jump: following,
				leave_read: self.channel.zip(self.seen_latest),
				..Self::default()
			};
		}
		if !self.initial_read_checked
			&& state.freshness == model::Freshness::Fresh
			&& !state.history_pending
			&& let Some(channel) = state.selected
			&& let Some(unread) = state.unread(channel)
		{
			self.initial_read_checked = true;
			if unread {
				self.mark_read = None;
				let arrived_on_live_edge = self.following
					&& state
						.channel(channel)
						.is_some_and(|channel| channel.supports_text())
					&& state.timeline.iter().next().is_some();
				if arrived_on_live_edge {
					self.hold_read_ack = true;
				}
			}
		}
		let can_load_newer = state.can_load_newer();
		if state
			.selected
			.is_some_and(|channel| state.missed(channel) == Some(false))
		{
			self.hold_read_ack = false;
		}
		if self.unread_jump || self.load_newer {
			self.browse_away();
		}
		// Incoming messages being watched at the live edge are not a new unread section.
		// Keep service read state unchanged while its acknowledgement is in flight.
		let watching_latest = self.following
			&& self.at_current_latest
			&& state.live_edge_latest().is_some()
			&& ui.input(|input| input.focused);
		let first_unread = state
			.selected
			.and_then(|channel| state.read_marker(channel))
			.and_then(|read| {
				state
					.timeline
					.iter()
					.find(|m| read.is_none_or(|id| m.id > id))
					.map(|m| m.id)
			});
		let kept = self
			.unread_boundary
			.filter(|id| state.timeline.get(*id).is_some());
		// The divider is fixed for the visit. Acknowledging moves the read marker to the latest
		// message, so the next arrival would otherwise look like a new first unread message;
		// only older unread history loading above it may move the divider up. Once the reader
		// has read past it, messages that then arrive unseen start a new section.
		let unseen_after_read = first_unread.filter(|first| {
			!watching_latest
				&& self
					.read_through()
					.is_some_and(|read| kept.is_some_and(|kept| kept <= read) && *first > read)
		});
		let boundary = match kept {
			Some(_) if unseen_after_read.is_some() => unseen_after_read,
			Some(kept) => Some(first_unread.map_or(kept, |first| first.min(kept))),
			None => first_unread.filter(|_| !watching_latest),
		};
		if self.unread_boundary != boundary {
			self.unread_boundary = boundary;
			self.revision = u64::MAX;
		}
		let text_size = egui::TextStyle::Body.resolve(ui.style()).size;
		let font_revision = crate::fonts::revision(ui.ctx());
		let scale = ui.ctx().pixels_per_point();
		let width_changed = (self.width - width).abs() > 1.0;
		let content_dimensions_changed = self.text_size != text_size
			|| self.font_revision != font_revision
			|| self.scale != scale
			|| self.hide_media_links != self.applied_hide_media_links
			|| self.compact_messages != self.applied_compact_messages;
		let dimensions_changed = width_changed || content_dimensions_changed;
		self.applied_hide_media_links = self.hide_media_links;
		self.applied_compact_messages = self.compact_messages;
		// A thread's starter joins the rows only once the whole thread history is loaded.
		let starter = state.thread_starter().filter(|_| {
			state.freshness == model::Freshness::Fresh
				&& !state.history_pending
				&& !state.history_targeted
				&& state.history_before.is_none()
				&& state.history_after.is_none()
				&& state.older_exhausted
		});
		let starter_id = starter.map(|m| m.id);
		let mut layout_changed = false;
		if self.revision != state.revision {
			// Member/presence and other unrelated updates must not restore the scroll
			// anchor or reintroduce estimates for already settled message geometry.
			let mut fingerprint = DefaultHasher::new();
			let mut previous = None;
			for message in starter.into_iter().chain(display_rows(state)) {
				let deleted = state.timeline.is_deleted(message.id);
				message.id.hash(&mut fingerprint);
				row_key(message, previous, self.unread_boundary, state).hash(&mut fingerprint);
				deleted.hash(&mut fingerprint);
				previous = Some(message);
			}
			let fingerprint = fingerprint.finish();
			layout_changed = self.layout_fingerprint != fingerprint;
			self.layout_fingerprint = fingerprint;
		}
		let changed = self.revision == u64::MAX
			|| layout_changed
			|| dimensions_changed
			|| self.starter_row != starter_id;
		self.starter_row = starter_id;
		if changed || self.revision != state.revision || self.width != width {
			self.measured_rows.clear();
		}
		let mut offset = None;
		let mut lead_rows = None;
		if changed {
			if content_dimensions_changed {
				self.heights.clear();
				self.pending_heights.clear();
			}
			// Keep measured heights as estimates during resize. Visible rows are
			// remeasured below; resetting everything makes the scroll extent jump.
			self.width = width;
			self.text_size = text_size;
			self.font_revision = font_revision;
			self.scale = scale;
			let row_ids: Vec<_> = starter_id
				.into_iter()
				.chain(display_rows(state).map(|message| message.id))
				.collect();
			self.heights
				.retain(|id, _| row_ids.binary_search(id).is_ok());
			self.suppressed_deleted_highlight
				.retain(|id| row_ids.binary_search(id).is_ok());
			self.formatted
				.retain(|id| display_message(state, id).is_some());
			self.toolbar = self
				.toolbar
				.filter(|(id, _)| display_message(state, *id).is_some());
			self.revealed.retain(|id, content| {
				display_message(state, *id).is_some_and(|m| content.matches(m))
			});
			let mut previous = None;
			let mut lead_basis = 0.0;
			let mut stale_heights = Vec::new();
			self.rows = starter
				.into_iter()
				.chain(display_rows(state))
				.map(|m| {
					let prior = previous;
					let deleted = state.timeline.is_deleted(m.id);
					let key = row_height_key(m, prior, self.unread_boundary, state, deleted);
					previous = Some(m);
					let lines = m
						.content
						.lines()
						.map(|line| {
							(line.chars().count() as f32 / ((width - 80.0) / 8.0).max(1.0))
								.ceil()
								.max(1.0)
						})
						.sum::<f32>();
					let mut estimate = (if m.embeds_suppressed {
						0.0
					} else {
						crate::embeds::estimated_height(&m.embeds)
							+ crate::invites::estimated_height(m)
					}) + crate::attachments::estimated_height(
						&m.attachments,
						(width - 88.0).max(1.0),
					) + 58.0 + 18.0 * lines.min(128.0);
					if self.compact_messages || grouped(prior, m, self.unread_boundary) {
						estimate = (estimate - GROUPED_ROW_SAVINGS).max(24.0);
					}
					estimate += reserved_chrome(ui, m, width);
					let height = match self.heights.get(&m.id) {
						Some((old_key, height)) if *old_key == key => *height,
						Some(_) => {
							stale_heights.push(m.id);
							estimate
						}
						None => estimate,
					};
					lead_basis += if height * 8.0 < estimate {
						estimate
					} else {
						height
					};
					(m.id, height)
				})
				.collect();
			for id in stale_heights {
				self.heights.remove(&id);
			}
			lead_rows = Some(lead_basis);
			if !self.following
				&& let Some((id, inset)) = self.anchor
			{
				offset = Some(anchor_offset(&self.rows, id, inset));
			}
		}
		self.revision = state.revision;
		let history_available = state
			.selected
			.is_some_and(|channel| state.can_read_history(channel));
		let empty = state.timeline.display_iter().next().is_none()
			&& !state
				.pending
				.iter()
				.any(|p| Some(p.channel) == state.selected);
		// An empty historical page or a pending refresh is not proof of a new channel.
		let welcome = empty
			&& history_available
			&& state.freshness == model::Freshness::Fresh
			&& !state.history_pending
			&& !state.history_targeted
			&& state.history_before.is_none()
			&& state.history_after.is_none()
			&& state.older_exhausted
			&& state
				.selected
				.and_then(|id| state.channel(id))
				.is_some_and(|channel| channel.guild.is_some() && channel.supports_text());
		if !history_available {
			ui.weak(crate::i18n::translate(
				"timeline-show-with-scroll-message-history-is-unavailable-with-current-permission-information",
			));
		}
		if history_available && state.freshness == model::Freshness::Loading && empty {
			let area = ui.available_rect_before_wrap().intersect(ui.clip_rect());
			loading_messages(ui);
			session.bind(ui, ui.scope_id().with(("timeline", state.selected)), area);
			let latest = state
				.selected
				.and_then(|channel| state.channel(channel))
				.and_then(|channel| channel.last_message);
			if state.show_missed_banner() && self.unread_dismissed != Some(latest) {
				let (jump_unread, mark_read) =
					unread_banner(ui, banner_rect(area), state.can_jump_unread(), None);
				if jump_unread {
					self.unread_jump = true;
					self.browse_away();
				}
				if jump_unread || mark_read {
					self.unread_dismissed = Some(latest);
				}
				if mark_read {
					self.hold_read_ack = false;
					self.mark_channel_read = state.selected;
				}
			}
			return;
		} else if empty && history_available && !welcome {
			ui.label(crate::i18n::translate_if_key(match state.freshness {
				model::Freshness::Loading => "timeline-show-with-scroll-loading-messages",
				model::Freshness::Unavailable => {
					"timeline-show-with-scroll-you-cannot-view-this-conversation"
				}
				model::Freshness::Stale => {
					"timeline-show-with-scroll-history-is-not-available-yet-use-reload-to-try-again"
				}
				model::Freshness::Fresh => {
					"timeline-show-with-scroll-no-messages-yet-start-the-conversation-below"
				}
			}));
		}
		let area = ui.available_rect_before_wrap().intersect(ui.clip_rect());
		let autoscroll_delta =
			session.bind(ui, ui.scope_id().with(("timeline", state.selected)), area);
		if autoscroll_delta > 0.0 {
			self.following = false;
		}
		let total: f32 = self.rows.iter().map(|(_, height)| height).sum();
		// The typing indicator floats in the reserved strip above the composer; the gap keeps
		// it from covering the last message, and stays there when nobody is typing.
		let end_padding = 8.0 + crate::typing::OVERLAY_HEIGHT;
		self.pending_heights.retain(|nonce, _| {
			state
				.pending
				.iter()
				.any(|p| &p.nonce == nonce && Some(p.channel) == state.selected)
		});
		let pending_rows: Vec<_> = state
			.pending
			.iter()
			.filter(|p| Some(p.channel) == state.selected)
			.map(|p| {
				let height = self
					.pending_heights
					.get(&p.nonce)
					.copied()
					.unwrap_or_else(|| {
						80.0 + p
							.content
							.lines()
							.map(|line| {
								(line.chars().count() as f32 / ((width - 88.0) / 8.0).max(1.0))
									.ceil()
									.max(1.0) * 20.0
							})
							.sum::<f32>() + if p.attachments.is_empty() { 0.0 } else { 320.0 }
					});
				self.pending_heights
					.entry(p.nonce.clone())
					.or_insert(height);
				(p, height)
			})
			.collect();
		let pending_height = pending_rows.iter().map(|(_, height)| *height).sum::<f32>();
		let packed = total + pending_height + end_padding;
		let lead_packed = lead_rows.unwrap_or(total) + pending_height + end_padding;
		if state.freshness == model::Freshness::Fresh && !state.history_pending {
			if let Some(target) = state.search_target.take() {
				let saved = state
					.restore_scroll
					.then(|| {
						state
							.selected
							.and_then(|channel| state.reading(channel))
							.filter(|cursor| cursor.message == Some(target))
					})
					.flatten();
				state.restore_scroll = false;
				let reveal = self.pending_reveal.take().unwrap_or(TargetReveal::Center);
				if state.timeline.get(target).is_some() {
					if saved.is_none() {
						self.highlighted = Some((target, ui.input(|input| input.time) + 2.0));
					}
					let viewport_h = area.height();
					let current_offset = self
						.scroll_offset
						.clamp(0.0, (packed - viewport_h).max(0.0));
					let (first, end, _) =
						visible_range(&self.rows, current_offset, current_offset + viewport_h);
					let visible = self.rows[first..end].iter().any(|(id, _)| *id == target);
					let stay = saved.is_none()
						&& match reveal {
							TargetReveal::StayIfVisible => visible,
							TargetReveal::Center => false,
						};
					if !stay {
						self.target_browsing = true;
						self.mark_read = None;
						self.following = false;
						self.jump = false;
						let to = if let Some(cursor) = saved {
							anchor_offset(&self.rows, target, cursor.inset)
								.clamp(0.0, (packed - viewport_h).max(0.0))
						} else {
							centered_offset(&self.rows, target, viewport_h, packed)
						};
						if saved.is_some()
							|| self.instant_scrolling
							|| (to - current_offset).abs() < 1.0
						{
							offset = Some(to);
							self.reveal_scroll = None;
						} else {
							self.reveal_scroll = Some(RevealScroll {
								target,
								from: current_offset,
								elapsed: 0.0,
							});
						}
					}
				} else {
					self.target_browsing = true;
					self.mark_read = None;
					state.status =
						"Message was not returned; it may have been removed or become unavailable";
				}
			} else {
				state.restore_scroll = false;
			}
		}
		if let Some((_, until)) = self.highlighted {
			let remaining = until - ui.input(|input| input.time);
			if remaining > 0.0 {
				ui.ctx()
					.request_repaint_after(std::time::Duration::from_secs_f64(remaining));
			} else {
				self.highlighted = None;
			}
		}
		let mut scroll = egui::ScrollArea::vertical()
			.id_salt(("timeline", state.selected))
			.auto_shrink([false, false])
			.animated(!self.instant_scrolling)
			.stick_to_bottom(self.following);
		let live_edge_offset =
			(total + end_padding + pending_rows.iter().map(|(_, height)| height).sum::<f32>()
				- ui.available_height())
			.max(0.0);
		let mut jumped_to = None;
		if std::mem::take(&mut self.jump) && self.following {
			self.reveal_scroll = None;
			self.present_scroll = None;
			offset = Some(live_edge_offset);
			jumped_to = Some(live_edge_offset);
		}
		let wheel = ui.input(|input| input.smooth_scroll_delta());
		let user_scroll = wheel.y + autoscroll_delta;
		if user_scroll != 0.0 && self.reveal_scroll.take().is_some() {
			offset = None;
		}
		if user_scroll != 0.0 {
			self.present_scroll = None;
		}
		// Jump to present glides back to the live edge instead of teleporting there.
		if let Some((from, elapsed)) = &mut self.present_scroll {
			*elapsed += ui.input(|input| input.stable_dt).clamp(1.0 / 240.0, 0.05);
			let t = *elapsed / PRESENT_SCROLL_SECS;
			if t >= 1.0 {
				offset = Some(live_edge_offset);
				self.present_scroll = None;
				self.follow_latest(state);
				self.jump = false;
			} else {
				offset = Some(*from + (live_edge_offset - *from) * ease_out_cubic(t));
			}
			ui.ctx().request_repaint();
		}
		if let Some(motion) = &mut self.reveal_scroll {
			motion.elapsed += ui.input(|input| input.stable_dt).clamp(1.0 / 240.0, 0.05);
			let t = motion.elapsed / REVEAL_SCROLL_SECS;
			let to = centered_offset(&self.rows, motion.target, area.height(), packed);
			if t >= 1.0 {
				offset = Some(to);
				self.reveal_scroll = None;
			} else {
				offset = Some(motion.from + (to - motion.from) * ease_out_cubic(t));
				ui.ctx().request_repaint();
			}
		}
		if autoscroll_delta != 0.0 {
			// Set the viewport before virtualization. A global scroll delta can be consumed
			// by nested embed scroll areas and moves past the rows laid out this frame.
			let max_offset = live_edge_offset;
			let next =
				(offset.unwrap_or(self.scroll_offset) - autoscroll_delta).clamp(0.0, max_offset);
			offset = Some(next);
			if next != self.scroll_offset {
				ui.ctx().request_repaint();
			}
		}
		if let Some(offset) = offset {
			scroll = scroll.vertical_scroll_offset(offset);
		}
		self.visible_authors.clear();
		#[cfg(test)]
		{
			self.leading_rendered = 0;
		}
		let mut measurements = Vec::new();
		let mut selected_reply = None;
		// ScrollArea consumes wheel input while applying it; retain the viewing gesture.
		let scroll_delta = wheel.y + autoscroll_delta;
		let allow_hover = !session.holding()
			&& !ui.input(|input| input.is_scrolling())
			&& ui.ctx().dragged_id().is_none();
		let output = scroll.show_viewport(ui, |ui, viewport| {
			ui.spacing_mut().item_spacing.y = 0.0;
			if welcome && let Some(channel) = state.selected.and_then(|id| state.channel(id)) {
				channel_welcome(ui, channel, viewport.height() - end_padding);
			}
			let lead = if welcome {
				0.0
			} else {
				(viewport.height() - lead_packed).max(0.0)
			};
			ui.add_space(lead);
			let overscan = 100.0;
			let (first, _, top) = visible_range(
				&self.rows,
				(viewport.min.y - overscan - lead).max(0.0),
				(viewport.max.y + 100.0 - lead).max(0.0),
			);
			let (anchor, _, anchor_top) = visible_range(
				&self.rows,
				(viewport.min.y - lead).max(0.0),
				(viewport.max.y - lead).max(0.0),
			);
			let content_top = ui.cursor().top();
			let clip = ui.clip_rect();
			// Measure leading overscan without changing the visible rows or parent bounds.
			// Its new heights take effect together with anchor restoration next pass.
			// The scroll content's box is only one viewport tall. A child that starts
			// there and then skips to `top` lays those rows past the box, so they
			// collapse and the live edge snaps when the real height comes back.
			let origin = ui.cursor().min;
			let mut leading = ui.new_child(
				egui::UiBuilder::new()
					.id_salt("leading-measurements")
					.max_rect(egui::Rect::from_min_size(
						origin,
						egui::vec2(
							ui.available_width(),
							(top + viewport.height() + overscan).max(ui.available_height()),
						),
					)),
			);
			leading.set_clip_rect(clip.with_max_y(clip.min.y));
			leading.add_space(top);
			ui.add_space(anchor_top);
			let keyboard_focus = ui
				.memory(|m| m.focused())
				.and_then(|id| ui.ctx().read_response(id));
			let retained_toolbar = self.toolbar.filter(|(_, rect)| {
				egui::Popup::is_any_open(ui.ctx())
					|| keyboard_focus
						.as_ref()
						.is_some_and(|r| rect.contains_rect(r.rect))
			});
			let reuse_leading = keyboard_focus.is_none()
				&& retained_toolbar.is_none()
				&& !egui::Popup::is_any_open(ui.ctx())
				&& !crate::select::has_selection(ui.ctx())
				&& !ui.input(|input| input.pointer.any_down() || input.pointer.any_released());
			let mut end = first;
			for index in first..self.rows.len() {
				let row_id = ui.make_persistent_id(self.rows[index].0.0);
				let ui = if index < anchor {
					&mut leading
				} else {
					&mut *ui
				};
				if index >= anchor && ui.cursor().top() > content_top + viewport.max.y + 100.0 {
					break;
				}
				end = index + 1;
				let id = self.rows[index].0;
				if Some(id) == self.starter_row
					&& let Some(starter) = state.thread_starter()
				{
					let response = ui
						.scope_builder(egui::UiBuilder::new().scope_id(row_id), |ui| {
							starter_row(ui, starter, state, avatars, width);
						})
						.response;
					measurements.push((
						id,
						row_height_key(
							starter,
							None,
							self.unread_boundary,
							state,
							state.timeline.is_deleted(id),
						),
						response.rect.height(),
					));
					continue;
				}
				let can_mark_read = state.can_mark_read(id);
				let can_mark_unread = state.can_mark_unread(id);
				let Some(message) = display_message(state, id) else {
					continue;
				};
				if !message.author.webhook
					&& self.visible_authors.len() < client_core::member_search::LIMIT
					&& !self.visible_authors.contains(&message.author.id)
				{
					self.visible_authors.push(message.author.id);
				}
				let previous = index
					.checked_sub(1)
					.and_then(|i| display_message(state, self.rows[i].0));
				let deleted = state.timeline.is_deleted(id);
				// ponytail: reuse only settled ordinary text; dynamic media, references,
				// spoilers and reactions need explicit layout invalidation before caching.
				if index < anchor
					&& reuse_leading
					&& self.measured_rows.contains(&id)
					&& message.kind == 0
					&& !message.unsupported
					&& !message.extra_content.any()
					&& message.reply_to.is_none()
					&& message.interaction.is_none()
					&& message.attachments.is_empty()
					&& message.embeds.is_empty()
					&& message.components.is_empty()
					&& !message.content.contains(['<', '|', '/'])
					&& state
						.reactions
						.display(message)
						.is_some_and(<[_]>::is_empty)
					&& state.interactions.pending.is_none()
					&& let Some(&(key, height)) = self.heights.get(&id)
					&& key
						== row_height_key(message, previous, self.unread_boundary, state, deleted)
				{
					ui.add_space(height);
					// Keep one result per row: visible height updates below zip by index.
					measurements.push((id, key, height));
					continue;
				}
				#[cfg(test)]
				if index < anchor {
					self.leading_rendered += 1;
				}

				let compact =
					self.compact_messages || grouped(previous, message, self.unread_boundary);
				let new_day = previous
					.is_none_or(|previous| timestamp(previous.id).date() != timestamp(id).date());
				let response = ui.scope_builder(egui::UiBuilder::new().scope_id(row_id), |ui| {
					if new_day {
						let date = timestamp(id);
						divider(
							ui,
							format!("{} {}, {}", date.month(), date.day(), date.year()),
							false,
							self.compact_messages,
						);
					}
					if self.unread_boundary == Some(id) {
						divider(ui, "New messages".into(), true, self.compact_messages);
					}
					let colors = crate::design::palette(ui);
					let background = ui.painter().add(egui::Shape::Noop);
					let mut time_rect = None;
					// A reaction claims its own right click: the row menu must stay closed.
					let mut reaction_menu = false;
					// A double-click away from text, links and media uses the selected reaction preset.
					let mut double_click = None;
					let row = egui::Frame::NONE
						.inner_margin(egui::Margin {
							left: 16,
							right: 16,
							top: if self.compact_messages {
								3
							} else if compact {
								1
							} else {
								group_gap(self.compact_messages)
							},
							bottom: 1,
						})
						.show(ui, |ui| {
							let mut surface = crate::select::Surface::new(ui, "row");
							ui.spacing_mut().item_spacing =
								egui::vec2(16.0, if self.compact_messages { 2.0 } else { 4.0 });
							if let Some(interaction) = message
								.interaction
								.as_deref()
								.filter(|_| message.reply_to.is_none())
							{
								ui.horizontal(|ui| {
									ui.spacing_mut().interact_size.y = 18.0;
									ui.spacing_mut().item_spacing.x = 6.0;
									reference_spine(ui, &colors);
									let avatar =
										avatars.show(ui, &interaction.user, 16.0, state.demo);
									surface.keep(&avatar);
									let name = ui.add(
										egui::Label::new(
											RichText::new(&interaction.user.name)
												.size(13.0)
												.family(crate::design::semibold_family(ui.ctx()))
												.color(colors.text),
										)
										.truncate(),
									);
									surface.keep(&name);
									let used = ui.add(
										egui::Label::new(
											RichText::new(crate::i18n::translate(
												"timeline-command-used",
											))
											.size(13.0)
											.color(colors.muted),
										)
										.truncate(),
									);
									surface.keep(&used);
									let label = if interaction.command.is_empty() {
										crate::i18n::translate("timeline-command-a-command")
									} else {
										format!("/{}", interaction.command)
									};
									let command = egui::Frame::NONE
										.fill(colors.mention_bg)
										.corner_radius(3)
										.inner_margin(egui::Margin::symmetric(4, 0))
										.show(ui, |ui| {
											ui.add(
												egui::Label::new(
													RichText::new(label)
														.size(13.0)
														.family(crate::design::semibold_family(
															ui.ctx(),
														))
														.color(colors.mention_text),
												)
												.truncate(),
											)
										})
										.inner;
									surface.keep(&command);
								});
							}
							if let Some(reply) = message.reply_to {
								ui.horizontal(|ui| {
									ui.spacing_mut().interact_size.y = 18.0;
									ui.spacing_mut().item_spacing.x = 6.0;
									reference_spine(ui, &colors);
									// Reuse only loaded content; never fetch a thread while painting.
									if message.reply_deleted || state.timeline.is_deleted(reply) {
										let deleted = ui.add(
											egui::Label::new(
												RichText::new(crate::i18n::translate(
													"timeline-show-with-scroll-message-deleted",
												))
												.size(13.0)
												.italics()
												.color(colors.muted),
											)
											.truncate(),
										);
										surface.keep(&deleted);
									} else {
										ui.scope(|ui| {
											ui.visuals_mut().disabled_alpha = 1.0;
											ui.add_enabled_ui(
												state.can_open_reply_target(reply),
												|ui| {
													let mut preview =
														egui::text::LayoutJob::default();
													let mut preview_emojis = Vec::new();
													let mut attachment_icon = None;
													if let Some(original) =
														state.timeline.get(reply)
													{
														let reply_avatar = avatars.show(
															ui,
															&original.author,
															16.0,
															state.demo,
														);
														if reply_avatar.clicked() {
															self.request_reply_target(reply);
														}
														surface.keep(&reply_avatar);
														attachment_icon = (!original
															.attachments
															.is_empty())
														.then(|| {
															if original.attachments.iter().any(
																|attachment| {
																	attachment.is_image()
																		|| attachment.is_video()
																},
															) {
																crate::icons::Icon::Image
															} else {
																crate::icons::Icon::File
															}
														});
														preview.append(
															&format!(
																"@{}  ",
																state.message_author_name(original)
															),
															0.0,
															egui::TextFormat {
																font_id: egui::FontId::new(
																	13.0,
																	crate::design::semibold_family(
																		ui.ctx(),
																	),
																),
																color: colors.muted,
																..Default::default()
															},
														);
														if crate::embeds::has_spoilers(original) {
															preview.append(
																"Spoiler",
																0.0,
																egui::TextFormat {
																	font_id:
																		egui::FontId::proportional(
																			13.0,
																		),
																	color: colors.muted,
																	..Default::default()
																},
															);
														} else if attachment_icon.is_some()
															&& original.content.trim().is_empty()
														{
															preview.append(
																&crate::i18n::translate(
																	"timeline-show-with-scroll-click-to-see-attachment",
																),
																0.0,
																egui::TextFormat {
																	font_id:
																		egui::FontId::proportional(
																			13.0,
																		),
																	color: colors.muted,
																	italics: true,
																	..Default::default()
																},
															);
														} else {
															let source =
																crate::mentions::MentionSource {
																	state,
																	channel: original.channel,
																};
															let formatted = self
																.formatted
																.get(reply, &original.content);
															queue_missing_channel_reference(
																&mut self.channel_reference_load,
																formatted,
																state,
																u32::MAX,
															);
															preview_emojis = formatted
																.append_inline_preview(
																	&mut preview,
																	ui,
																	&original.mentions,
																	Some(&source),
																	crate::mentions::known_roles(
																		state,
																		original.channel,
																	),
																	&state.channels,
																);
														}
													} else {
														preview.append(
															"Earlier message · View original",
															0.0,
															egui::TextFormat {
																font_id: egui::FontId::proportional(
																	13.0,
																),
																color: colors.muted,
																..Default::default()
															},
														);
													}
													let accessible = Formatted::inline_preview_text(
														&preview,
														&preview_emojis,
													);
													// Keep room for the trailing attachment glyph when truncating.
													let icon_size = 16.0;
													let label_width = ui.available_width()
														- attachment_icon.map_or(0.0, |_| {
															icon_size + ui.spacing().item_spacing.x
														});
													let (
														preview_pos,
														preview_galley,
														reply_preview,
													) = ui.allocate_ui(
														egui::vec2(
															label_width.max(0.0),
															ui.available_height(),
														),
														|ui| {
															egui::Label::new(preview)
																.truncate()
																.sense(egui::Sense::click())
																.layout_in_ui(ui)
														},
													)
													.inner;
													if let Some(icon) = attachment_icon {
														crate::icons::inline(
															ui,
															icon,
															icon_size,
															colors.muted,
														);
													}
													reply_preview.widget_info(|| {
														egui::WidgetInfo::labeled(
															egui::Role::Link,
															ui.is_enabled(),
															&accessible,
														)
													});
													surface.embed(
														&reply_preview,
														preview_pos,
														preview_galley.clone(),
													);
													Formatted::paint_inline_preview_emojis(
														ui,
														preview_pos,
														&preview_galley,
														&preview_emojis,
													);
													let reply_preview = reply_preview
														.on_hover_cursor(
															egui::CursorIcon::PointingHand,
														)
														.on_hover_text(crate::i18n::translate(
															"timeline-show-with-scroll-view-original-message",
														))
														.on_disabled_hover_text(
															crate::i18n::translate(
																"timeline-show-with-scroll-wait-for-readable-current-message-history",
															),
														);
													surface.keep(&reply_preview);
													if reply_preview.clicked() {
														self.request_reply_target(reply);
													}
												},
											);
										});
									}
								});
							}
							let system = message.system_message();
							let author_colors = state.message_author_colors(message);
							let name_color = author_colors.map_or(colors.text_strong, |role| {
								crate::design::role_name_color(
									role.primary,
									colors.chat,
									colors.text_strong,
								)
							});
							let mut body_bottom = f32::NAN;
							// Hovering the avatar underlines the author, like hovering the name.
							let mut avatar_hot = false;
							ui.horizontal_top(|ui| {
								if system.is_some() {
									let (gutter, _) = ui.allocate_exact_size(
										egui::vec2(40.0, MESSAGE_LINE),
										egui::Sense::hover(),
									);
									let (icon, tint) = system_icon(message.kind, &colors);
									crate::icons::paint(
										ui.painter(),
										icon,
										egui::Rect::from_center_size(
											gutter.center() + egui::vec2(0.0, 1.5),
											egui::Vec2::splat(18.0),
										),
										tint,
									);
								} else if self.compact_messages {
									// Compact rows drop the avatar gutter: time, author, then
									// the body in its own column so wrapped lines never run under the name.
									let body_spacing = ui.spacing().item_spacing.x;
									ui.spacing_mut().item_spacing.x = 8.0;
									let time = timestamp(id);
									compact_header(
										ui,
										ui.available_width(),
										egui::FontId::proportional(12.0),
										|ui| {
											let time = ui
												.label(
													RichText::new(format!(
														"{:02}:{:02}",
														time.hour(),
														time.minute()
													))
													.size(12.0)
													.color(colors.muted),
												)
												.on_hover_text_with(|| format!("{} UTC", time));
											surface.exclude(time.rect);
										},
									);
									let width = compact_author_width(ui.available_width());
									compact_header(
										ui,
										width,
										egui::FontId::new(
											15.5,
											crate::design::medium_family(ui.ctx()),
										),
										|ui| {
											ui.set_max_width(width);
											let author = crate::account_badge::name(
												ui,
												&message.author,
												state.message_author_name(message),
												15.5,
												(colors.text_strong, author_colors, colors.chat),
												egui::Sense::click(),
												0.0,
											)
											.on_hover_cursor(egui::CursorIcon::PointingHand);
											if author.hovered() {
												ui.painter().hline(
													author.rect.x_range(),
													author.rect.bottom() - 1.0,
													egui::Stroke::new(1.0, name_color),
												);
											}
											crate::user_menu::show(
												&author,
												state,
												&message.author,
												profile,
												&mut self.user_action,
											);
											profile.person_click(
												ui,
												&author,
												None,
												&message.author,
											);
											surface.keep(&author);
										},
									);
									ui.spacing_mut().item_spacing.x = body_spacing;
								} else if compact {
									time_rect = Some(
										ui.allocate_exact_size(
											egui::vec2(40.0, MESSAGE_LINE),
											egui::Sense::hover(),
										)
										.0,
									);
								} else {
									let avatar = avatars
										.show(ui, &message.author, 40.0, state.demo)
										.on_hover_cursor(egui::CursorIcon::PointingHand);
									avatar_hot = avatar.hovered();
									crate::user_menu::show(
										&avatar,
										state,
										&message.author,
										profile,
										&mut self.user_action,
									);
									profile.person_click(ui, &avatar, None, &message.author);
									surface.keep(&avatar);
								}
								ui.vertical(|ui| {
									ui.set_width(ui.available_width());
									let mut text_line = egui::Rect::NOTHING;
									if !compact && system.is_none() {
										ui.allocate_ui_with_layout(
											egui::vec2(ui.available_width(), MESSAGE_LINE),
											egui::Layout::left_to_right(egui::Align::Center),
											|ui| {
												ui.spacing_mut().item_spacing.x = 8.0;
												let author = crate::account_badge::name(
													ui,
													&message.author,
													state.message_author_name(message),
													15.5,
													(
														colors.text_strong,
														author_colors,
														colors.chat,
													),
													egui::Sense::click(),
													48.0,
												)
												.on_hover_cursor(egui::CursorIcon::PointingHand);
												if avatar_hot || author.hovered() {
													let line = author.rect.bottom() - 1.0;
													ui.painter().hline(
														author.rect.x_range(),
														line,
														egui::Stroke::new(1.0, name_color),
													);
												}
												crate::user_menu::show(
													&author,
													state,
													&message.author,
													profile,
													&mut self.user_action,
												);
												profile.person_click(
													ui,
													&author,
													None,
													&message.author,
												);
												surface.keep(&author);
												let time = timestamp(id);
												let time = ui
													.label(
														RichText::new(format!(
															"{:02}:{:02}",
															time.hour(),
															time.minute()
														))
														.size(12.0)
														.color(colors.muted),
													)
													.on_hover_text_with(|| format!("{} UTC", time));
												surface.exclude(time.rect);
											},
										);
									}
									if let Some(system) = &system {
										// Only a thread-start row carries the thread affordances.
										let thread = (message.kind == 18 && system.content_shown)
											.then(|| {
												(
													state.thread_of(message).map(|c| c.id),
													message.channel,
												)
											});
										show_system(
											ui,
											system,
											timestamp(id),
											state,
											profile,
											&mut self.user_action,
											&mut surface,
											thread,
											(
												&mut self.channel_reference,
												&mut self.threads_request,
											),
										);
									}
									let body = egui::Frame::NONE
										.inner_margin(egui::Margin {
											left: if message.forwarded { 16 } else { 0 },
											..Default::default()
										})
										.show(ui, |ui| {
											if message.forwarded {
												ui.label(
													RichText::new(crate::i18n::translate(
														"timeline-show-with-scroll-forwarded",
													))
													.size(13.0)
													.italics()
													.color(colors.muted),
												);
												ui.add_space(4.0);
											}
											let body_color = if deleted
												&& !self.suppressed_deleted_highlight.contains(&id)
											{
												Some(colors.danger)
											} else {
												None
											};
											let formatted =
												self.formatted.get(id, &message.content);
											let reveal = self
												.revealed
												.get(&id)
												.filter(|reveal| reveal.matches(message));
											let before = reveal.map_or((0, false), |reveal| {
												(reveal.text, reveal.media)
											});
											let mut text =
												if formatted.spoilers { before.0 } else { 0 };
											queue_missing_channel_reference(
												&mut self.channel_reference_load,
												formatted,
												state,
												text,
											);
											let mut media = before.1;
											let content_shown =
												system.as_ref().is_some_and(|s| s.content_shown);
											if !content_shown
												&& !(self.hide_media_links
													&& crate::embeds::standalone_media_links(
														message,
													)) {
												let jumbo = formatted.jumbo();
												text_line = ui
													.scope(|ui| {
														if jumbo {
															crate::design::jumbo_emoji(ui);
														}
														if let Some(color) = body_color {
															ui.visuals_mut().override_text_color =
																Some(color);
														}
														if deleted && message.content.is_empty() {
															ui.label(
																RichText::new(
																	crate::i18n::translate(
																		"timeline-show-with-scroll-deleted-message-had-no-text",
																	),
																)
																.size(16.0)
																.color(
																	body_color
																		.unwrap_or(colors.text),
																),
															);
														} else {
															let source =
																crate::mentions::MentionSource {
																	state,
																	channel: message.channel,
																};
															formatted.show_references(
																ui,
																&mut self.opening,
																&message.mentions,
																Some(&source),
																profile,
																(
																	&state.channels,
																	&mut self.channel_reference,
																	&state.guilds,
																	crate::mentions::known_roles(
																		state,
																		message.channel,
																	),
																),
																(avatars, state.demo, &mut text),
																&mut surface,
																crate::design::MessageCardSurface::Conversation,
															);
														}
													})
													.response
													.rect;
											}
											if formatted.limited {
												ui.label(
													RichText::new(crate::i18n::translate(
														"timeline-show-with-scroll-display-limited-copy-message-for-the-full-text",
													))
													.small()
													.color(colors.muted),
												);
											}
											if crate::embeds::has_media_spoilers(message) && !media
											{
												let reveal = ui.button(crate::i18n::translate(
													"timeline-show-with-scroll-reveal-spoiler-media",
												));
												surface.keep(&reveal);
												if reveal.clicked() {
													media = true;
												}
											} else {
												let invite_top = ui.cursor().top();
												crate::invites::show(
													ui,
													message,
													state,
													avatars,
													&mut self.invite_requests,
													&mut self.invite_action,
												);
												surface.exclude(egui::Rect::from_min_max(
													egui::pos2(ui.max_rect().left(), invite_top),
													egui::pos2(
														ui.max_rect().right(),
														ui.min_rect().bottom(),
													),
												));
												let embed_top = ui.cursor().top();
												if let Some(gif) = crate::embeds::show(
													ui,
													message,
													&mut self.formatted,
													avatars,
													&mut self.opening,
													&mut self.download,
													profile,
													state,
													crate::design::MessageCardSurface::Conversation,
												) {
													self.gif_favorite = Some(gif);
												}
												surface.exclude(egui::Rect::from_min_max(
													egui::pos2(ui.max_rect().left(), embed_top),
													egui::pos2(
														ui.max_rect().right(),
														ui.min_rect().bottom(),
													),
												));
												if !message.extra_content.components_v2 {
													let previous_view = self.viewing;
													crate::attachments::show(
														ui,
														message,
														avatars,
														&mut self.viewing,
														&mut self.opening,
														&mut self.download,
														&mut self.audio,
														&mut self.video,
														state.demo,
														&mut surface,
														crate::design::MessageCardSurface::Conversation,
													);
													if self.viewing != previous_view {
														self.component_viewing = None;
														self.embed_viewing = None;
													}
												}
											}
											if text != 0 || media {
												let hide = ui.small_button(crate::i18n::translate(
													"timeline-show-with-scroll-hide-spoilers",
												));
												surface.keep(&hide);
												if hide.clicked() {
													text = 0;
													media = false;
												}
											}
											if before != (text, media) {
												if text == 0 && !media {
													self.revealed.remove(&id);
												} else {
													self.revealed.insert(
														id,
														Revealed::new(message, text, media),
													);
												}
												self.heights.remove(&id);
												ui.ctx().request_repaint();
											}
											if message.edited {
												ui.label(
													RichText::new(crate::i18n::translate(
														"timeline-show-with-scroll-edited",
													))
													.small()
													.color(colors.muted),
												);
											}
											if !message.components.is_empty() {
												let shown = ui.scope(|ui| {
													self.components.show(
														ui,
														message,
														state,
														avatars,
														&mut self.opening,
														&mut crate::components::MediaUi {
															component_viewing: &mut self
																.component_viewing,
															viewing: &mut self.viewing,
															download: &mut self.download,
															audio: &mut self.audio,
															video: &mut self.video,
														},
													)
												});
												surface.exclude(shown.response.rect);
												if shown.inner.is_some() {
													self.component_action = shown.inner;
												}
											}
											if state.interactions.pending.as_ref().is_some_and(
												|pending| pending.message == Some(message.id),
											) {
												ui.small(crate::i18n::translate(
													"timeline-show-with-scroll-application-interaction-pending",
												));
											}
											if !message.components.is_empty()
												&& let Some(error) = state.interactions.error
											{
												ui.colored_label(colors.danger, error);
											}
											if message.ephemeral {
												ui.horizontal(|ui| {
													ui.spacing_mut().item_spacing.x = 4.0;
													let (icon, _) = ui.allocate_exact_size(
														egui::Vec2::splat(16.0),
														egui::Sense::hover(),
													);
													crate::icons::paint(
														ui.painter(),
														crate::icons::Icon::EyeSlash,
														icon,
														colors.muted,
													);
													ui.label(
														RichText::new(crate::i18n::translate(
															"timeline-show-with-scroll-only-you-can-see-this",
														))
														.size(13.0)
														.color(colors.muted),
													);
													let dismiss = ui
														.add(
															egui::Button::new(
																RichText::new(
																	crate::i18n::translate(
																		"timeline-show-with-scroll-dismiss-message",
																	),
																)
																.size(13.0)
																.color(colors.link),
															)
															.frame(false),
														)
														.on_hover_cursor(
															egui::CursorIcon::PointingHand,
														);
													surface.keep(&dismiss);
													if dismiss.clicked() {
														self.dismiss_ephemeral = Some(id);
													}
												});
											}

											for sticker in &message.sticker_items {
												let response = ui
													.push_id(sticker.id, |ui| {
														crate::stickers::message(
															ui,
															sticker,
															state,
															avatars,
															&mut self.sticker_request,
															&mut self.browse_sticker,
														)
													})
													.inner;
												surface.keep(&response);
											}
											if message.poll.is_some() {
												let shown = ui.scope(|ui| {
													self.polls.show(ui, state, message, avatars)
												});
												surface.exclude(shown.response.rect);
												if let Some(action) = shown.inner {
													self.poll_action = Some((message.id, action));
												}
											}
											let unknown_system = message.unsupported
												&& message.system_summary().is_none();
											if unknown_system
												|| (message.extra_content.poll
													&& message.poll.is_none()) || ((message
												.extra_content
												.sticker_items
												|| message.extra_content.stickers)
												&& message.sticker_items.is_empty()) || ((message
												.extra_content
												.components
												|| message.extra_content.components_v2)
												&& message.components.is_empty())
											{
												if unknown_system {
													ui.label(
														RichText::new(format!(
															"Unsupported message type {} · Preview unavailable",
															message.kind
														))
														.small()
														.color(colors.muted),
													);
												}
												for (present, label) in [
													(
														message.extra_content.poll
															&& message.poll.is_none(),
														"Poll · Preview unavailable",
													),
													(
														(message.extra_content.sticker_items
															|| message.extra_content.stickers)
															&& message.sticker_items.is_empty(),
														"Sticker · Preview unavailable",
													),
													(
														(message.extra_content.components
															|| message.extra_content.components_v2)
															&& message.components.is_empty(),
														"Components · Preview unavailable",
													),
												] {
													if present {
														ui.label(
															RichText::new(label)
																.small()
																.color(colors.muted),
														);
													}
												}
												let target = state
													.channels
													.iter()
													.find(|c| {
														c.id == message.channel
															&& state.can_view(c.id)
													})
													.and_then(|c| discord_url(c, Some(message.id)));
												let open = ui.add_enabled(
													target.is_some(),
													egui::Button::new(crate::i18n::translate(
														"timeline-show-with-scroll-open-in-discord",
													)),
												);
												surface.keep(&open);
												if open.clicked() {
													self.browser_opening = target;
												}
											}
										});
									body_bottom = body.response.rect.bottom();
									if message.forwarded {
										let rail = egui::Rect::from_min_max(
											body.response.rect.min,
											egui::pos2(
												body.response.rect.left() + 3.0,
												body.response.rect.bottom(),
											),
										);
										ui.painter().rect_filled(rail, 2.0, colors.selected);
									}
									// A "started a thread" row already links the thread inline.
									if system.is_none()
										&& let Some(thread) = state.thread_of(message)
									{
										let card = thread_card(ui, thread, &colors);
										surface.keep(&card);
										if card.clicked() {
											self.channel_reference = Some(thread.id);
										}
									}
									let reactions_top = ui.cursor().top();
									if deleted {
										crate::reactions::show_frozen(
											ui,
											state.reactions.display(message),
											(avatars, state.demo),
										);
									} else if let Some(action) = crate::reactions::show(
										ui,
										state.reactions.display(message),
										state.gateway_connected
											&& state.freshness == model::Freshness::Fresh
											&& state.can_read_history(message.channel),
										state.reactions.busy(),
										(state.history_pending && state.history_before.is_none())
											|| state.reactions.invalidated(message.id),
										(avatars, state.demo),
										message.id,
										state.reactions.users.as_ref(),
										|emoji, add| state.can_react(id, Some(emoji), add),
									) {
										match action {
											crate::reactions::Action::Reload => {
												self.reaction = Some((id, None));
											}
											crate::reactions::Action::Toggle(emoji) => {
												self.reaction = Some((id, Some(emoji)));
											}
											crate::reactions::Action::Inspect(emoji, open) => {
												reaction_menu |= open;
												self.reaction_users = Some((id, emoji, open));
											}
										}
									}
									surface.exclude(egui::Rect::from_min_max(
										egui::pos2(ui.max_rect().left(), reactions_top),
										egui::pos2(ui.max_rect().right(), ui.min_rect().bottom()),
									));
									fill_header_line(ui, compact, text_line);
								});
							});
							let mut cover = ui.min_rect();
							if body_bottom.is_finite() {
								cover.max.y = body_bottom;
							}
							double_click = ui
								.input(|input| {
									input
										.pointer
										.button_double_clicked(egui::PointerButton::Primary)
										.then(|| input.pointer.interact_pos())
										.flatten()
								})
								.filter(|pos| surface.blank_at(*pos));
							surface.cover(cover);
							surface.finish(ui);
						});
					let rect = row.response.rect;
					if let Some(pos) = double_click
						&& let Some(text) = self.double_click_reaction
						&& !deleted && rect.contains(pos)
						&& self
							.toolbar
							.is_none_or(|(_, toolbar)| !toolbar.contains(pos))
						&& ui.ctx().layer_id_at(pos) == Some(ui.layer_id())
						&& !egui::Popup::is_any_open(ui.ctx())
					{
						let (emoji, reacted, enabled) = quick_reaction(state, message, text);
						if enabled {
							// The second click selected the nearest word in the blank band.
							crate::select::clear(ui.ctx());
							self.reaction = Some((id, Some(emoji)));
							if !reacted {
								self.quick_reaction_used = Some(text);
							}
						}
					}
					let mentioned = mentions_viewer(message, state);
					// Mentions mark the row in the warning colour; a private command response in
					// the house accent, as in the official client.
					let marked = if mentioned {
						Some(colors.warning)
					} else if message.ephemeral {
						Some(colors.accent)
					} else {
						None
					};
					if let Some(mark) = marked {
						ui.painter().set(
							background,
							egui::Shape::rect_filled(
								rect,
								0.0,
								crate::design::row_highlight(ui, mark, 0.10),
							),
						);
						ui.painter().rect_filled(
							egui::Rect::from_min_size(rect.min, egui::vec2(3.0, rect.height())),
							0.0,
							mark,
						);
					}
					let focus = ui.interact(
						rect,
						ui.scope_id().with("message-focus"),
						egui::Sense::focusable_noninteractive(),
					);
					focus.widget_info(|| {
						egui::WidgetInfo::labeled(
							egui::Role::Label,
							true,
							format!(
								"Message by {}. {}Tab for actions.",
								message.author.name,
								if mentioned { "Mentions you. " } else { "" },
							),
						)
					});
					let retained = retained_toolbar.is_some_and(|(active, _)| active == id);
					// The floating toolbar overlaps the row above; pointer inside it keeps this row active.
					let toolbar_hover = self
						.toolbar
						.filter(|(active, toolbar)| {
							*active == id && ui.rect_contains_pointer(*toolbar)
						})
						.is_some();
					let other_toolbar_hover = self
						.toolbar
						.filter(|(active, toolbar)| {
							*active != id && ui.rect_contains_pointer(*toolbar)
						})
						.is_some();
					let hovered = allow_hover
						&& (ui.rect_contains_pointer(rect) || toolbar_hover)
						&& !other_toolbar_hover
						&& !egui::Popup::is_any_open(ui.ctx())
						&& retained_toolbar.is_none_or(|(active, _)| active == id);
					let context_menu = (ui.rect_contains_pointer(rect) || toolbar_hover)
						&& !other_toolbar_hover
						&& !reaction_menu && !egui::Popup::is_any_open(ui.ctx())
						&& (ui.input(|i| i.pointer.secondary_clicked())
							|| crate::select::open_menu(ui.ctx()));
					if context_menu
						|| hovered || focus.has_focus()
						|| keyboard_focus.as_ref().is_some_and(|r| r.id == focus.id)
						|| retained
					{
						ui.painter().set(
							background,
							egui::Shape::rect_filled(
								rect,
								0.0,
								if let Some(mark) = marked {
									crate::design::row_highlight(ui, mark, 0.16)
								} else {
									crate::design::row_highlight(ui, colors.hover, 0.7)
								},
							),
						);
						if let Some(rect) = time_rect {
							let time = timestamp(id);
							ui.painter().text(
								rect.center(),
								egui::Align2::CENTER_CENTER,
								format!("{:02}:{:02}", time.hour(), time.minute()),
								egui::FontId::proportional(11.0),
								colors.muted,
							);
							ui.interact(
								rect,
								ui.scope_id().with("timestamp"),
								egui::Sense::hover(),
							)
							.on_hover_text_with(|| format!("{} UTC", time));
						}
						let own = state
							.user
							.as_ref()
							.is_some_and(|u| u.id == message.author.id);
						if deleted {
							let toolbar_rect = egui::Rect::from_min_size(
								egui::pos2(rect.right() - 46.0, rect.top() - 10.0),
								egui::vec2(36.0, 28.0),
							);
							let mut toolbar = ui.new_child(
								egui::UiBuilder::new()
									.id_salt("hover-actions")
									.max_rect(toolbar_rect)
									.layout(egui::Layout::left_to_right(egui::Align::Center)),
							);
							toolbar.spacing_mut().item_spacing = egui::vec2(2.0, 0.0);
							toolbar.spacing_mut().button_padding = egui::vec2(4.0, 2.0);
							toolbar.spacing_mut().interact_size.y = 28.0;
							toolbar
								.painter()
								.rect_filled(toolbar_rect, 6.0, colors.raised);
							toolbar.painter().rect_stroke(
								toolbar_rect,
								6.0,
								egui::Stroke::new(1.0, colors.border),
								egui::StrokeKind::Inside,
							);
							let menu = action_button(
								&mut toolbar,
								crate::icons::Icon::More,
								"profiles-show-more",
							);
							menu.widget_info(|| {
								egui::WidgetInfo::labeled(
									egui::Role::Button,
									toolbar.is_enabled(),
									crate::i18n::translate_args(
										"timeline-deleted-message-actions-for",
										&[("user", &message.author.name)],
									),
								)
							});
							let mut popup = egui::Popup::menu(&menu);
							if context_menu {
								popup = popup.open_memory(Some(egui::SetOpenCommand::Bool(true)));
							}
							if context_menu
								|| (!menu.clicked()
									&& egui::Popup::position_of_id(toolbar.ctx(), popup.get_id())
										.is_some())
							{
								popup = popup.at_pointer_fixed();
							}
							let mut action = None;
							deleted_message_actions(popup, &mut action);
							match action {
								Some(DeletedLocalAction::ToggleHighlight) => {
									if !self.suppressed_deleted_highlight.remove(&id) {
										self.suppressed_deleted_highlight.insert(id);
									}
								}
								Some(DeletedLocalAction::Remove) => {
									self.remove_preserved = Some(id);
								}
								None => {}
							}
							self.toolbar = Some((id, toolbar_rect));
						} else {
							let react = state.can_react(id, None, true)
								|| message.reactions.as_ref().is_some_and(|items| {
									items
										.iter()
										.any(|r| state.can_react(id, Some(&r.emoji), true))
								});
							// Holding Shift exposes direct actions alongside the menu.
							let shift = ui.input(|input| input.modifiers.shift);
							let can_pin = state.can_pin(message.channel, id);
							let can_delete = state.can_delete(message.channel, id);
							let link = state
								.channel(message.channel)
								.and_then(|channel| discord_url(channel, Some(id)));
							let buttons = if react && !shift { 4 } else { 1 }
								+ 3 + usize::from(own) + if shift {
								2 + usize::from(link.is_some())
									+ usize::from(can_delete) + usize::from(can_mark_unread)
									+ usize::from(can_pin)
							} else {
								0
							};
							let width = buttons as f32 * 30.0;
							let toolbar_rect = egui::Rect::from_min_size(
								egui::pos2(
									(rect.right() - 16.0 - width).max(rect.left()),
									rect.top() - 10.0,
								),
								egui::vec2(width, 28.0),
							);
							// A child overlay keeps hover from changing wrapping or cached row heights.
							let mut toolbar = ui.new_child(
								egui::UiBuilder::new()
									.id_salt("hover-actions")
									.max_rect(toolbar_rect)
									.layout(egui::Layout::left_to_right(egui::Align::Center)),
							);
							toolbar.spacing_mut().item_spacing = egui::vec2(2.0, 0.0);
							toolbar.spacing_mut().button_padding = egui::vec2(4.0, 2.0);
							toolbar.spacing_mut().interact_size.y = 28.0;
							toolbar
								.painter()
								.rect_filled(toolbar_rect, 6.0, colors.raised);
							toolbar.painter().rect_stroke(
								toolbar_rect,
								6.0,
								egui::Stroke::new(1.0, colors.border),
								egui::StrokeKind::Inside,
							);
							if react && !shift {
								let quick = self
									.quick_reactions
									.unwrap_or(crate::reactions::QUICK_DEFAULTS);
								let mut last = egui::Rect::NOTHING;
								for (index, text) in quick.into_iter().enumerate() {
									let (emoji, reacted, enabled) =
										quick_reaction(state, message, text);
									let response = toolbar
										.push_id(("quick-reaction", index), |ui| {
											ui.add_enabled_ui(enabled, |ui| {
												crate::reactions::quick_button(ui, text, reacted)
											})
											.inner
										})
										.inner;
									last = response.rect;
									if response.clicked() {
										self.reaction = Some((id, Some(emoji)));
										if !reacted {
											self.quick_reaction_used = Some(text);
										}
									}
								}
								let x = last.right() + 1.0;
								toolbar.painter().vline(
									x,
									last.y_range().shrink(6.0),
									egui::Stroke::new(1.0, colors.border),
								);
							}
							if let Some((anchor, trigger)) = crate::reactions::add_button(
								&mut toolbar,
								react,
								state.reactions.busy(),
							) {
								self.reaction_picker = Some((id, anchor, trigger));
							}
							let can_reply = state.can_send(message.channel) && !message.ephemeral;
							let can_edit =
								!message.unsupported && state.can_edit(message.channel, id);
							if toolbar
								.add_enabled_ui(can_reply, |ui| {
									action_button(
										ui,
										crate::icons::Icon::Reply,
										"timeline-message-actions-reply",
									)
								})
								.inner
								.clicked()
							{
								selected_reply = Some(id);
							}
							if toolbar
								.add_enabled_ui(state.can_forward(id), |ui| {
									action_button(
										ui,
										crate::icons::Icon::Forward,
										"timeline-message-actions-forward",
									)
								})
								.inner
								.clicked()
							{
								self.forward_request = Some(id);
							}
							if own
								&& toolbar
									.add_enabled_ui(can_edit, |ui| {
										action_button(
											ui,
											crate::icons::Icon::Pencil,
											"timeline-message-actions-edit-message",
										)
									})
									.inner
									.clicked()
							{
								*editing = Some((message.channel, id, message.content.clone()));
								self.edit_started = true;
							}
							if shift {
								let pinned = state.is_pinned(message.channel, id);
								if can_pin
									&& action_button(
										&mut toolbar,
										crate::icons::Icon::Pin,
										if pinned {
											"timeline-message-actions-unpin-message"
										} else {
											"timeline-message-actions-pin-message"
										},
									)
									.clicked()
								{
									self.pin_request = Some((message.channel, id, !pinned));
								}
								if can_mark_unread
									&& action_button(
										&mut toolbar,
										crate::icons::Icon::Inbox,
										"timeline-message-actions-mark-unread",
									)
									.clicked()
								{
									self.mark_unread = Some(id);
								}
								if let Some(link) = &link {
									copy_action_button(
										&mut toolbar,
										crate::icons::Icon::Link,
										"timeline-message-actions-copy-message-link",
										egui::Id::unique(("copy-link", state.generation, id)),
										&mut self.copy_feedback,
										|| link.clone(),
									);
								}
								copy_action_button(
									&mut toolbar,
									crate::icons::Icon::Copy,
									"message-menu-copy",
									egui::Id::unique(("copy-message", state.generation, id)),
									&mut self.copy_feedback,
									|| message.display_text().into_owned(),
								);
								copy_action_button(
									&mut toolbar,
									crate::icons::Icon::Hash,
									"timeline-message-actions-copy-message-id",
									egui::Id::unique(("copy-id", state.generation, id)),
									&mut self.copy_feedback,
									|| id.to_string(),
								);
							}
							if can_delete
								&& shift && toolbar
								.push_id("quick-delete", |ui| {
									action_button(
										ui,
										crate::icons::Icon::Trash,
										"timeline-message-actions-delete-message",
									)
								})
								.inner
								.clicked()
							{
								self.quick_delete = Some((message.channel, id));
							}
							let menu = action_button(
								&mut toolbar,
								crate::icons::Icon::More,
								"profiles-show-more",
							);
							menu.widget_info(|| {
								egui::WidgetInfo::labeled(
									egui::Role::Button,
									toolbar.is_enabled(),
									crate::i18n::translate_args(
										"timeline-message-actions-for",
										&[("user", &message.author.name)],
									),
								)
							});
							let mut popup = egui::Popup::menu(&menu).id(egui::Id::unique((
								"message-menu",
								state.generation,
								id,
							)));
							if context_menu {
								popup = popup.open_memory(Some(egui::SetOpenCommand::Bool(true)));
							}
							if context_menu
								|| (!menu.clicked()
									&& egui::Popup::position_of_id(toolbar.ctx(), popup.get_id())
										.is_some())
							{
								popup = popup.at_pointer_fixed();
							}
							message_actions(
								popup,
								(
									message,
									link.as_deref(),
									&self.extension_actions,
									&mut self.extension_request,
								),
								(own, can_reply, can_edit, can_delete),
								(
									can_mark_read.then_some(&mut self.mark_read),
									can_mark_unread.then_some(&mut self.mark_unread),
									&mut selected_reply,
								),
								(editing, &mut self.edit_started),
								deleting,
								(
									can_pin,
									state.is_pinned(message.channel, id),
									&mut self.pin_request,
								),
								(
									state.can_create_thread(message.channel)
										&& state.thread_of(message).is_none(),
									&mut self.thread_request,
								),
								(state.can_forward(id), &mut self.forward_request),
								message
									.reactions
									.as_ref()
									.filter(|_| state.can_read_history(message.channel))
									.and_then(|items| items.first())
									.map(|reaction| {
										(reaction.emoji.clone(), &mut self.reaction_users)
									}),
							);
							self.toolbar = Some((id, toolbar_rect));
						}
					}
					if selected_reply.or(state.reply_target()) == Some(id)
						|| self.highlighted.is_some_and(|(target, _)| target == id)
					{
						ui.painter().set(
							background,
							egui::Shape::rect_filled(
								rect,
								0.0,
								crate::design::row_highlight(ui, colors.accent, 0.25),
							),
						);
					}
				});
				measurements.push((
					id,
					row_height_key(message, previous, self.unread_boundary, state, deleted),
					response.response.rect.height(),
				));
			}
			let used: f32 = self.rows[..end].iter().map(|(_, height)| *height).sum();
			ui.add_space((total - used).max(0.0));
			for (index, (pending, height)) in pending_rows.iter().enumerate() {
				let compact = index > 0
					|| state.timeline.iter().next_back().is_some_and(|previous| {
						let now = crate::local_time::now();
						state
							.user
							.as_ref()
							.is_some_and(|user| user.id == previous.author.id)
							&& !previous.unsupported
							&& !previous.extra_content.any()
							&& timestamp(previous.id).date() == now.date()
							&& (now - timestamp(previous.id)).whole_seconds() < 300
					});
				let top = ui.cursor().top() - content_top;
				if top + height < viewport.min.y - 100.0 || top > viewport.max.y + 100.0 {
					ui.add_space(*height);
					continue;
				}
				let response = ui.push_id(("pending", &pending.nonce), |ui| {
					crate::pending::show(
						ui,
						pending,
						(
							compact,
							group_gap(self.compact_messages),
							self.compact_messages,
						),
						state,
						(
							avatars,
							&mut self.opening,
							profile,
							&mut self.channel_reference,
							&mut self.pending_formatted,
						),
						upload,
						(&mut self.restore_pending, &mut self.cancel_upload),
					);
				});
				let measured = response.response.rect.height();
				if (measured - height).abs() > 1.0 {
					ui.ctx().request_discard("Pending message height settled");
					ui.ctx().request_repaint();
				}
				self.pending_heights.insert(pending.nonce.clone(), measured);
			}
			// Only the end of the conversation has extra space; it scrolls with the messages.
			ui.add_space(end_padding);
			// Visible rows occupy their measured height immediately; leading overscan
			// still occupies its old height until the next anchored pass.
			for (index, (_, _, height)) in (first..end).zip(&measurements) {
				if index >= anchor {
					self.rows[index].1 = *height;
				}
			}
			viewport.min.y
		});
		self.scroll_offset = output.state.clamped_offset().y;
		if jumped_to.is_some_and(|target| (self.scroll_offset - target).abs() > 1.0) {
			self.jump = false;
			ui.ctx().request_discard("Timeline live edge settled");
		}
		// ScrollArea applies wheel input after laying out its contents. Preserve that
		// movement when new row measurements rebuild the timeline on the next pass.
		let spare = if welcome {
			0.0
		} else {
			(output.inner_rect.height() - lead_packed).max(0.0)
		};
		let lead = spare;
		let (anchor, _, anchor_top) = visible_range(
			&self.rows,
			(output.state.clamped_offset().y - lead).max(0.0),
			(output.state.clamped_offset().y + output.inner_rect.height() - lead).max(0.0),
		);
		self.anchor = self
			.rows
			.get(anchor)
			.map(|(id, _)| (*id, output.state.clamped_offset().y - lead - anchor_top));
		if selected_reply.is_some() {
			state.reply = selected_reply.map(client_core::Reply::to);
			self.reply_started = true;
		}
		let distance_from_bottom =
			(output.content_size.y - output.state.clamped_offset().y - output.inner_rect.height())
				.max(0.0);
		let whole_conversation_visible =
			state.older_exhausted && packed <= output.inner_rect.height() + 3.0;
		let at_bottom = distance_from_bottom <= 3.0 || whole_conversation_visible;
		// The live edge counts even when service latest metadata outlived a deleted message;
		// otherwise the unread banners could never resolve for that channel.
		self.at_current_latest = state.live_edge_latest().is_some()
			|| state.timeline.iter().next_back().is_some_and(|message| {
				state.channels.iter().any(|channel| {
					Some(channel.id) == state.selected && channel.last_message == Some(message.id)
				})
			});
		let channel_latest = state
			.selected
			.and_then(|channel| state.channel(channel))
			.and_then(|channel| channel.last_message);
		let scrolled_toward_bottom = ui.input(|input| {
			(scroll_delta < 0.0
				&& (session.holding()
					|| input
						.pointer
						.hover_pos()
						.is_some_and(|pos| output.inner_rect.contains(pos))))
				|| (input.pointer.any_down() && output.state.clamped_offset().y > output.inner)
		});
		if at_bottom && (can_load_newer || self.at_current_latest) {
			if can_load_newer {
				if scrolled_toward_bottom {
					self.load_newer = true;
					self.browse_away();
					if autoscroll_delta == 0.0 {
						ui.ctx().request_repaint();
					}
				}
			} else if scrolled_toward_bottom {
				self.target_browsing = false;
				self.hold_read_ack = false;
				// Scrolling down to the live edge reads the section, so its banner goes away.
				self.unread_dismissed = Some(channel_latest);
				if state.history_targeted || state.history_after.is_some() {
					self.latest = true;
				}
				if autoscroll_delta == 0.0 {
					ui.ctx().request_repaint();
				}
			}
		}
		let was_following = self.following;
		self.following = at_bottom && !self.target_browsing;
		if self.following
			&& self.at_current_latest
			&& !state.history_targeted
			&& state.history_after.is_none()
			&& ui.input(|i| i.focused)
			&& let Some(latest) = state.live_edge_latest()
		{
			self.seen_latest = Some(latest);
			// A focused reader at the live edge reads each arrival as it lands: nothing they
			// watched may raise the banner or add to its count. An unacknowledged join keeps
			// its banner until the reader scrolls toward the bottom or marks it read.
			if !self.hold_read_ack && ui.is_enabled() {
				self.unread_dismissed = Some(channel_latest);
			}
		}
		if self.following
			&& !self.hold_read_ack
			&& ui.is_enabled()
			&& self.mark_unread.is_none()
			&& !state.history_targeted
			&& state.history_after.is_none()
			&& ui.input(|i| i.focused)
			&& let Some(latest) = state.live_edge_latest()
			&& self.auto_read_attempt != Some(latest)
			&& self.at_current_latest
			&& state.can_mark_read(latest)
		{
			// One automatic attempt per viewed latest message; failed ACKs remain manually retryable.
			self.auto_read_attempt = Some(latest);
			self.mark_read = Some(latest);
		}
		let mut reflow = false;
		for (id, key, height) in measurements {
			self.measured_rows.insert(id);
			if self
				.heights
				.get(&id)
				.is_none_or(|(old_key, old)| *old_key != key || (*old - height).abs() > 1.0)
			{
				self.heights.insert(id, (key, height));
				reflow = true;
			}
		}
		if reflow {
			self.reflow_frames = self.reflow_frames.saturating_add(1);
			self.consecutive_reflows = self.consecutive_reflows.saturating_add(1);
			self.revision = u64::MAX;
			let user_scrolling = scroll_delta != 0.0 || session.holding();
			if was_following && !user_scrolling {
				self.following = true;
				self.jump = true;
				// An anchored reader keeps this frame's places. The next frame
				// applies the new leading height through the scroll anchor.
				if !dimensions_changed || channel_changed {
					ui.ctx().request_discard("Timeline message heights settled");
				}
			}
			ui.ctx().request_repaint();
		}
		if !reflow {
			self.consecutive_reflows = 0;
		}
		// Explicit upward input requests one page even when a short view cannot scroll.
		// Idle layout still never drains history merely to fill the viewport.
		self.load_older = output.state.clamped_offset().y < 160.0
			&& ui.input(|i| {
				scroll_delta > 0.0
					&& (session.holding()
						|| i.pointer
							.hover_pos()
							.is_some_and(|pos| output.inner_rect.contains(pos)))
			}) && state.can_load_older();
		if self.load_older {
			self.browse_away();
		}
		// Discord-style overlays: an unread strip hangs from the top edge, the typing indicator
		// floats in the reserved strip above the composer, and a round control offers the way
		// back to the live edge. They are painted after the scroll area so they sit above the
		// messages and win the hit-test.
		let area = output.inner_rect;
		let now = std::time::Instant::now();
		let typing = state
			.selected
			.filter(|channel| crate::typing::active(state, *channel, now));
		// The bottom edge fades into the composer: one continuous ramp from nothing down to the
		// chat surface, with no flat band anywhere in it. While someone is typing the ramp runs
		// tall enough to carry the indicator, and denser once the reader has scrolled away from
		// the live edge, so the line stays legible over the messages behind it.
		//
		// A see-through surface (window transparency or a background image) cannot hide messages
		// without also tinting what shows through, so its ramp only ever reaches the surface's
		// own tint, and at the live edge, where the reserved strip is empty, it is skipped.
		let surface = crate::design::section_surface(
			ui,
			crate::design::window_palette(ui).chat,
			crate::design::ImageSection::MessageList,
		);
		let see_through = surface.a() < 255;
		let fade_height = if see_through && self.following {
			0.0
		} else if typing.is_some() {
			crate::typing::OVERLAY_HEIGHT + 8.0
		} else {
			20.0
		};
		let dense = if see_through {
			surface
		} else if self.following {
			surface.gamma_multiply(0.88)
		} else {
			surface.to_opaque()
		};
		let fade_rect = egui::Rect::from_min_max(
			egui::pos2(area.left(), (area.bottom() - fade_height).max(area.top())),
			area.right_bottom(),
		);
		let mut fade = egui::Mesh::default();
		// Stacked strips approximate a smooth ease instead of a straight ramp, which reads as a
		// visible edge where it starts.
		const FADE_STEPS: usize = 12;
		for step in 0..=FADE_STEPS {
			let t = step as f32 / FADE_STEPS as f32;
			let y = fade_rect.top() + fade_rect.height() * t;
			let color = dense.gamma_multiply(t * t * (3.0 - 2.0 * t));
			fade.colored_vertex(egui::pos2(fade_rect.left(), y), color);
			fade.colored_vertex(egui::pos2(fade_rect.right(), y), color);
			if step > 0 {
				let base = (step as u32 - 1) * 2;
				fade.add_triangle(base, base + 1, base + 3);
				fade.add_triangle(base, base + 3, base + 2);
			}
		}
		ui.painter()
			.with_clip_rect(area)
			.add(egui::Shape::mesh(fade));
		if let Some(channel) = typing {
			crate::typing::overlay(
				ui,
				egui::Rect::from_min_max(
					egui::pos2(
						area.left() + 16.0,
						(area.bottom() - crate::typing::OVERLAY_HEIGHT).max(area.top()),
					),
					egui::pos2(area.right() - 16.0, area.bottom()),
				),
				state,
				channel,
				now,
			);
		}
		let browsing_history = state.history_targeted
			|| state.history_before.is_some()
			|| state.history_after.is_some();
		let missed = state.show_missed_banner();
		let opening_unread =
			missed && state.freshness == model::Freshness::Loading && state.history_pending;
		let raise_unread = missed
			&& (state.timeline.iter().next().is_some() || opening_unread)
			&& (opening_unread
				|| self.hold_read_ack
				|| !(self.following
					&& self.at_current_latest
					&& state.live_edge_latest().is_some()
					&& ui.input(|input| input.focused)));
		// Once raised, the banner stays with its divider for the rest of the visit, unless the
		// reader dismissed it and nothing newer has arrived since.
		let dismissed = self.unread_dismissed == Some(channel_latest);
		let kept_unread = self.unread_session && self.unread_boundary.is_some() && !dismissed;
		let show_unread = (raise_unread || kept_unread) && !dismissed;
		let can_jump_unread = show_unread && (state.can_jump_unread() || kept_unread);
		// Latest-message metadata can outlive a deleted message. A complete, visible
		// latest page has nowhere useful to jump; targeted pages still need navigation.
		// Older history needs no bar of its own: the round control leads back to the present.
		if show_unread
			&& (!whole_conversation_visible
				|| browsing_history
				|| opening_unread
				|| self.hold_read_ack
				|| kept_unread)
		{
			self.unread_session |= self.unread_boundary.is_some();
			let summary = self.unread_summary(state);
			let (jump_unread, mark_read) =
				unread_banner(ui, banner_rect(area), can_jump_unread, summary.as_deref());
			if jump_unread {
				if state.can_jump_unread() {
					self.unread_jump = true;
					self.browse_away();
				} else if let Some(boundary) = self.unread_boundary {
					// The section was acknowledged during this visit; its divider is still loaded.
					self.request_reply_target(boundary);
				}
				// The reader is at the section now; only later arrivals bring the banner back.
				self.unread_dismissed = Some(channel_latest);
			}
			if mark_read {
				self.unread_dismissed = Some(channel_latest);
				self.hold_read_ack = false;
				self.mark_channel_read = state.selected;
			}
		}
		// Older pages appended to the live timeline keep their cursor after loading; that
		// alone must not raise the bar the moment a reader nudges upward. Only pages that are
		// detached from the live edge (targeted or forward history) show it immediately.
		let detached_page = state.history_targeted || state.history_after.is_some();
		let unread = state
			.selected
			.is_some_and(|channel| state.missed(channel) == Some(true));
		// Messages that landed at the live edge after the reader last had it on screen.
		let new_below = if self.following || browsing_history || !self.at_current_latest {
			0
		} else {
			self.seen_latest
				.or(self.unread_dismissed.flatten())
				.map_or(0, |seen| {
					state.timeline.iter().filter(|m| m.id > seen).count()
				})
		};
		self.present_control = None;
		if (!self.following && distance_from_bottom > PRESENT_CONTROL_SCREENS * area.height())
			|| (self.target_browsing && !(at_bottom && unread))
			|| detached_page
			|| new_below > 0
		{
			// A control on the right edge, not a bar across the conversation: it says the
			// same thing with far less furniture and never covers a message being read.
			let label = (new_below > 0).then(|| {
				crate::i18n::translate_count(
					"timeline-present-control-new-messages",
					new_below,
					&[],
				)
			});
			let (rect, present) = present_control(ui, area, unread, label.as_deref());
			self.present_control = Some(rect);
			if present {
				if browsing_history {
					self.latest = true;
				}
				if distance_from_bottom > 0.5 && !browsing_history && !self.instant_scrolling {
					// Glide back so the reader keeps their place in the conversation.
					self.target_browsing = false;
					self.present_scroll = Some((output.state.clamped_offset().y, 0.0));
				} else {
					self.follow_latest(state);
				}
				ui.ctx().request_repaint();
			}
		}
		if let Some((message_id, attachment_id)) = self.pending_viewer
			&& state.timeline.get(message_id).is_some()
		{
			self.pending_viewer = None;
			self.embed_viewing = None;
			self.viewing = Some((message_id, attachment_id));
		}
		self.show_fullscreen_video(ui.ctx(), state);
		if let Some((message, media)) = self.download.embed_view_request.take()
			&& let Some(source) = display_message(state, message)
		{
			self.viewing = None;
			self.component_viewing = None;
			self.embed_viewing = Some((
				message,
				Revealed::fingerprint(source),
				model::Attachment {
					id: Id(0),
					filename: "Embed image.png".into(),
					description: None,
					content_type: Some("image/png".into()),
					size: 0,
					media,
					spoiler: false,
					duration_ms: None,
					waveform: Vec::new(),
				},
			));
		}
		if let Some((message, fingerprint, image)) = &self.embed_viewing {
			let allowed = !state.timeline.is_deleted(*message)
				&& display_message(state, *message).is_some_and(|source| {
					!source.embeds_suppressed
						&& Revealed::fingerprint(source) == *fingerprint
						&& (!crate::embeds::has_media_spoilers(source)
							|| self
								.revealed
								.get(message)
								.is_some_and(|reveal| reveal.media && reveal.matches(source)))
				});
			if !allowed
				|| crate::attachments::viewer(
					ui,
					std::slice::from_ref(image),
					image.id,
					avatars,
					&mut self.download,
					&mut self.opening,
					state.demo,
				)
				.is_none()
			{
				self.embed_viewing = None;
			}
		}
		if let Some((message_id, attachment_id)) = self.viewing {
			let message = state
				.timeline
				.get(message_id)
				.or_else(|| {
					state
						.interactions
						.ephemeral
						.iter()
						.find(|message| message.id == message_id)
				})
				.filter(|m| {
					if let Some(identity) = self.component_viewing {
						identity
							== (
								m.id,
								egui::Id::unique((&m.components, &m.attachments)).value(),
							)
					} else {
						!crate::embeds::has_media_spoilers(m)
							|| self
								.revealed
								.get(&m.id)
								.is_some_and(|reveal| reveal.media && reveal.matches(m))
					}
				});
			self.viewing = message.and_then(|m| {
				crate::attachments::viewer(
					ui,
					if self.component_viewing.is_some() {
						m.attachments
							.iter()
							.find(|a| a.id == attachment_id)
							.map(std::slice::from_ref)
							.unwrap_or(&[])
					} else {
						&m.attachments
					},
					attachment_id,
					avatars,
					&mut self.download,
					&mut self.opening,
					state.demo,
				)
				.map(|id| (message_id, id))
			});
		}
	}
}
/// Offline exercise of unread navigation, reply history and returning to the live edge.
#[cfg(feature = "demo")]
pub fn debug_unread_navigation_check(state: &mut State) {
	use client_core::{Command, Envelope, Event, read_state};
	let channel = state.selected.unwrap();
	let template = state.timeline.iter().next().unwrap().clone();
	let message = |id| Message {
		poll: None,
		id: Id(id),
		content: format!("Synthetic message {id}"),
		reply_to: None,
		..template.clone()
	};
	let apply = |state: &mut State, event| {
		state.apply(Envelope {
			generation: state.generation,
			event,
		});
	};
	let page = |state: &mut State, start, end| {
		apply(
			state,
			Event::History {
				channel,
				request: state.request,
				older: state.history_before.is_some(),
				messages: (start..=end).map(&message).collect(),
			},
		);
	};
	let ctx = egui::Context::default();
	let mut view = TimelineView {
		instant_scrolling: true,
		..Default::default()
	};
	let frame = |view: &mut TimelineView, state: &mut State| {
		for _ in 0..5 {
			ctx.run_ui(
				egui::RawInput {
					focused: true,
					screen_rect: Some(egui::Rect::from_min_size(
						egui::Pos2::ZERO,
						egui::vec2(900.0, 600.0),
					)),
					..Default::default()
				},
				|ui| {
					view.show_with_scroll(
						ui,
						state,
						&mut None,
						&mut None,
						(
							&mut crate::avatars::Avatars::default(),
							&mut crate::profiles::ProfileSession::default(),
						),
						None,
						&mut crate::scroll::Session::default(),
					);
				},
			)
			.drop_without_applying_deltas();
		}
	};
	state.history(None);
	page(state, 451, 500);
	state
		.apply_read_state(read_state::Event::Ack {
			channel,
			message: Some(Id(499)),
			manual: true,
			mention_count: Some(0),
			version: None,
		})
		.unwrap();
	frame(&mut view, state);
	let count = state.timeline.row_count();
	assert!(
		state.open_unread().is_none(),
		"Loaded unread must scroll locally"
	);
	assert_eq!(state.search_target, Some(Id(500)));
	assert_eq!(state.timeline.row_count(), count);
	view.browse_away();
	frame(&mut view, state);
	assert_eq!(view.highlighted.map(|(id, _)| id), Some(Id(500)));
	view.follow_latest(state);
	frame(&mut view, state);
	assert_eq!(view.mark_read.take(), Some(Id(500)));
	let Command::MarkRead { request, .. } = state.prepare_mark_read(Id(500)).unwrap() else {
		panic!("Expected read acknowledgement");
	};
	state
		.apply_read_state(read_state::Event::Result {
			channel,
			message: Id(500),
			request,
			result: Ok(()),
		})
		.unwrap();

	// Opening an unloaded reply preserves the read marker and only offers history navigation.
	state.reply = Some(client_core::Reply::to(Id(100)));
	assert!(state.open_reply_target(Id(100)).is_some());
	page(state, 51, 100);
	frame(&mut view, state);
	assert_eq!(state.missed(channel), Some(false));
	assert!(!state.show_missed_banner());
	assert!(state.can_load_newer() && view.present_control.is_some());
	assert!(view.mark_read.is_none());
	let Command::History { after, .. } = state.newer_history().unwrap() else {
		panic!("History");
	};
	assert_eq!(after, Some(Id(100)));
	page(state, 101, 150);
	apply(
		state,
		Event::Delete {
			channel,
			id: Id(150),
		},
	);
	let Command::History { after, .. } = state.newer_history().unwrap() else {
		panic!("History");
	};
	assert_eq!(
		after,
		Some(Id(150)),
		"Deleted trailing rows must not rewind pagination"
	);

	// Older pagination must not disable acknowledgement at the retained live edge.
	state.history(None);
	page(state, 451, 500);
	view.follow_latest(state);
	frame(&mut view, state);
	assert!(state.older_history().is_some());
	page(state, 401, 450);
	apply(state, Event::Message(message(501)));
	frame(&mut view, state);
	assert_eq!(view.mark_read.take(), Some(Id(501)));
	assert_eq!(state.live_edge_latest(), Some(Id(501)));
	apply(state, Event::Message(message(502)));

	// Once the bounded window evicts its newest end, live arrivals cannot bridge the gap.
	for end in (50..=400).rev().step_by(50) {
		assert!(state.older_history().is_some());
		page(state, end - 49, end);
	}
	assert!(state.history_targeted);
	assert!(state.live_edge_latest().is_none());
	apply(
		state,
		Event::Delete {
			channel,
			id: Id(100),
		},
	);
	apply(state, Event::Message(message(503)));
	assert!(state.timeline.get(Id(503)).is_none());
	view.follow_latest(state);
	assert!(
		view.latest,
		"Sending from detached history must request the present page"
	);
	assert_eq!(state.missed(channel), Some(true));
	assert_eq!(
		centered_offset(&[(Id(1), 2000.0)], Id(1), 600.0, 2000.0),
		0.0
	);
}

#[cfg(test)]
#[path = "pending_tests.rs"]
mod pending_tests;
#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn short_history_loads_older_only_after_explicit_upward_input() {
		let mut state = loading_unread_channel(false);
		state.freshness = model::Freshness::Fresh;
		state.history_pending = false;
		let current_id = ((1_788_998_580_000u64 - 1_420_070_400_000) << 22) | 1;
		let mut current = text_message(current_id);
		current.content = "Existing synthetic message".into();
		state.channels[0].last_message = Some(Id(current_id));
		let older_id = current_id - (86_400_000u64 << 22);
		state
			.apply_read_state(client_core::read_state::Event::Snapshot {
				entries: Some(vec![(Id(20), Some(Id(older_id)), 0)]),
				version: Some(2),
				partial: false,
			})
			.unwrap();
		state.timeline.insert(current, false, false).unwrap();
		assert!(state.can_load_older());
		let ctx = egui::Context::default();
		let mut view = TimelineView::default();
		for _ in 0..5 {
			banner_frame(&ctx, &mut view, &mut state, vec![], false);
			assert!(
				!view.load_older,
				"idle short history must not drain older pages"
			);
		}
		let initial = banner_frame(&ctx, &mut view, &mut state, vec![], false);
		let initial_y = initial
			.iter()
			.find(|(text, _)| text == "Existing synthetic message")
			.unwrap()
			.1
			.top();
		let wheel = |delta| {
			vec![
				egui::Event::PointerMoved(egui::pos2(400.0, 300.0)),
				egui::Event::MouseWheel {
					unit: egui::MouseWheelUnit::Point,
					phase: egui::TouchPhase::Move,
					source: egui::MouseWheelSource::Unknown,
					delta: egui::vec2(0.0, delta),
					modifiers: egui::Modifiers::NONE,
				},
			]
		};
		banner_frame(&ctx, &mut view, &mut state, wheel(-60.0), false);
		assert!(!view.load_older);
		banner_frame(&ctx, &mut view, &mut state, wheel(120.0), false);
		assert!(
			view.load_older,
			"upward scroll must reach history even when rows fit in one view"
		);
		assert!(!view.following);
		assert!(state.older_history().is_some());
		for _ in 0..3 {
			banner_frame(&ctx, &mut view, &mut state, vec![], false);
			assert!(
				!view.following && view.target_browsing,
				"waiting for a page preserves browsing intent"
			);
		}
		banner_frame(&ctx, &mut view, &mut state, wheel(120.0), false);
		assert!(
			!view.load_older,
			"pending history must suppress duplicate requests"
		);
		// End the gesture before the response so buffered wheel motion is not
		// mistaken for history restoration. The unread boundary also stays on
		// the existing message, rather than changing its row's chrome.
		banner_frame(
			&ctx,
			&mut view,
			&mut state,
			vec![egui::Event::MouseWheel {
				unit: egui::MouseWheelUnit::Point,
				phase: egui::TouchPhase::End,
				source: egui::MouseWheelSource::Unknown,
				delta: egui::Vec2::ZERO,
				modifiers: egui::Modifiers::NONE,
			}],
			false,
		);
		assert!(!view.following && view.target_browsing);
		let mut older = text_message(older_id);
		older.content = "Synthetic older text\n\n".repeat(40);
		state.apply(client_core::Envelope {
			generation: state.generation,
			event: client_core::Event::History {
				channel: Id(20),
				request: state.request,
				older: true,
				messages: vec![older],
			},
		});
		assert!(!state.history_pending);
		for _ in 0..4 {
			banner_frame(&ctx, &mut view, &mut state, vec![], false);
			assert!(
				!view.following && view.target_browsing,
				"an arriving older page must not jump to the live edge"
			);
		}
		let restored = banner_frame(&ctx, &mut view, &mut state, vec![], false);
		let restored_y = restored
			.iter()
			.find(|(text, _)| text == "Existing synthetic message")
			.unwrap()
			.1
			.top();
		assert!(
			(initial_y - restored_y).abs() <= 2.0,
			"older history preserves the message position: {initial_y} -> {restored_y}"
		);
		view.follow_latest(&state);
		assert!(
			view.following && !view.target_browsing,
			"explicit latest navigation still resumes following"
		);
	}

	#[cfg(target_os = "macos")]
	#[test]
	fn macos_control_click_opens_the_existing_message_menu() {
		let ctx = egui::Context::default();
		crate::design::apply(&ctx);
		let mut state = loading_unread_channel(false);
		state.freshness = model::Freshness::Fresh;
		state.history_pending = false;
		state
			.timeline
			.insert(text_message(20), false, false)
			.unwrap();
		let mut view = TimelineView::default();
		let mut labels = Vec::new();
		for _ in 0..5 {
			labels = banner_frame(&ctx, &mut view, &mut state, vec![], false);
		}
		let pos = labels
			.iter()
			.find(|(label, _)| label.contains("Synthetic text"))
			.expect("visible message")
			.1
			.center();
		for pressed in [true, false] {
			labels = banner_frame(
				&ctx,
				&mut view,
				&mut state,
				vec![
					egui::Event::PointerMoved(pos),
					egui::Event::PointerButton {
						pos,
						button: egui::PointerButton::Primary,
						pressed,
						modifiers: egui::Modifiers {
							ctrl: pressed,
							..Default::default()
						},
					},
				],
				false,
			);
		}
		for _ in 0..3 {
			labels = banner_frame(&ctx, &mut view, &mut state, vec![], false);
		}
		assert!(
			labels
				.iter()
				.any(|(label, _)| label == &crate::i18n::translate("message-menu-copy")),
			"Control-click must open the message menu: {labels:?}"
		);
		assert!(!view.reply_started && state.reply.is_none());
	}

	// Synthetic regressions: no transport or acknowledgement worker is running.
	fn banner_frame(
		ctx: &egui::Context,
		view: &mut TimelineView,
		state: &mut State,
		events: Vec<egui::Event>,
		shift_widget_order: bool,
	) -> Vec<(String, egui::Rect)> {
		banner_frame_bounds(ctx, view, state, events, shift_widget_order, false)
	}

	fn banner_frame_bounds(
		ctx: &egui::Context,
		view: &mut TimelineView,
		state: &mut State,
		events: Vec<egui::Event>,
		shift_widget_order: bool,
		actual_glyphs: bool,
	) -> Vec<(String, egui::Rect)> {
		banner_frame_bounds_width(
			ctx,
			view,
			state,
			events,
			shift_widget_order,
			actual_glyphs,
			900.0,
		)
	}

	fn banner_frame_bounds_width(
		ctx: &egui::Context,
		view: &mut TimelineView,
		state: &mut State,
		events: Vec<egui::Event>,
		shift_widget_order: bool,
		actual_glyphs: bool,
		width: f32,
	) -> Vec<(String, egui::Rect)> {
		fn collect(
			shape: &egui::Shape,
			labels: &mut Vec<(String, egui::Rect)>,
			actual_glyphs: bool,
		) {
			match shape {
				egui::Shape::Text(text) => {
					let mut rect = text.galley.rect.translate(text.pos.to_vec2());
					if actual_glyphs {
						// Wrapped labels include the occupied author space in their galley
						// rectangle; inspect actual first-row glyphs for painted text positions.
						if let Some(row) = text.galley.rows.first()
							&& let Some(glyph) = row.glyphs.first()
						{
							rect.min.x = text.pos.x + row.pos.x + glyph.pos.x;
						}
					}
					labels.push((text.galley.job.text.clone(), rect));
				}
				egui::Shape::Vec(shapes) => {
					for shape in shapes {
						collect(shape, labels, actual_glyphs);
					}
				}
				_ => {}
			}
		}
		let output = ctx.run_ui(
			egui::RawInput {
				focused: true,
				events,
				screen_rect: Some(egui::Rect::from_min_size(
					egui::Pos2::ZERO,
					egui::vec2(width, 600.0),
				)),
				..Default::default()
			},
			|ui| {
				// A conditional sibling changes auto IDs without moving the bar.
				if shift_widget_order {
					ui.skip_ahead_auto_ids(1);
				}
				view.show(
					ui,
					state,
					&mut None,
					&mut None,
					(
						&mut crate::avatars::Avatars::default(),
						&mut crate::profiles::ProfileSession::default(),
					),
					None,
				);
			},
		);
		assert!(
			output
				.platform_output
				.commands
				.iter()
				.all(|command| matches!(command, egui::OutputCommand::TextSelectionSettled(_))),
			"{:?}",
			output.platform_output.commands
		);
		let mut labels = vec![];
		for shape in &output.shapes {
			collect(&shape.shape, &mut labels, actual_glyphs);
		}
		output.drop_without_applying_deltas();
		labels
	}

	fn loading_unread_channel(cached_reply: bool) -> State {
		let mut state = State {
			auth: client_core::auth::AuthState::Authenticated,
			gateway_connected: true,
			freshness: model::Freshness::Loading,
			history_pending: true,
			history_targeted: false,
			history_before: None,
			history_after: None,
			older_exhausted: false,
			selected: Some(Id(20)),
			channels: vec![model::Channel {
				id: Id(20),
				guild: None,
				parent_id: None,
				position: 0,
				name: "Synthetic unread conversation".into(),
				kind: 1,
				recipients: vec![],
				member_list_id: None,
				tags: None,
				message_count: None,
				icon: None,
				last_message: Some(Id(20)),
			}],
			..Default::default()
		};
		state
			.apply_read_state(client_core::read_state::Event::Snapshot {
				entries: Some(vec![(Id(20), Some(Id(10)), 0)]),
				version: Some(1),
				partial: false,
			})
			.unwrap();
		if cached_reply {
			let mut message = text_message(20);
			message.reply_to = Some(Id(9));
			message.reactions = Some(vec![model::Reaction {
				emoji: model::ReactionEmoji {
					id: None,
					name: Some("wave".into()),
				},
				count: 1,
				me: false,
				me_burst: false,
			}]);
			state.timeline.insert(message, false, false).unwrap();
		}
		state
	}

	#[test]
	fn embed_lightbox_reuses_viewer_and_closes_when_source_is_hidden_or_changed() {
		for reason in [0, 1, 2] {
			let mut state = loading_unread_channel(false);
			state.freshness = model::Freshness::Fresh;
			state.history_pending = false;
			let mut message = text_message(20);
			let media = model::EmbedMedia {
				url: Some("https://cdn.discordapp.com/attachments/1/2/synthetic.png".into()),
				width: 100,
				height: 100,
				..Default::default()
			};
			message.embeds.push(model::Embed {
				kind: "image".into(),
				image: Some(media.clone()),
				description: (reason == 2).then(|| "||Synthetic spoiler||".into()),
				..Default::default()
			});
			message.attachments.push(model::Attachment {
				id: Id(10),
				filename: "Synthetic image.png".into(),
				description: None,
				content_type: Some("image/png".into()),
				size: 64,
				media: media.clone(),
				spoiler: false,
				duration_ms: None,
				waveform: vec![],
			});
			state
				.timeline
				.insert(message.clone(), false, false)
				.unwrap();
			let ctx = egui::Context::default();
			let mut view = TimelineView::default();
			for _ in 0..4 {
				banner_frame(&ctx, &mut view, &mut state, vec![], false);
			}
			if reason == 2 {
				view.download.view_embed(message.id, &media);
				banner_frame(&ctx, &mut view, &mut state, vec![], false);
				assert!(view.embed_viewing.is_none(), "unrevealed media never opens");
				view.revealed
					.insert(message.id, Revealed::new(&message, 0, true));
			}
			view.download.view_embed(message.id, &media);
			banner_frame(&ctx, &mut view, &mut state, vec![], false);
			assert!(view.embed_viewing.is_some());
			assert!(view.opening.is_none() && view.download.request.is_none());
			view.pending_viewer = Some((message.id, Id(10)));
			banner_frame(&ctx, &mut view, &mut state, vec![], false);
			assert!(
				view.embed_viewing.is_none() && view.viewing == Some((message.id, Id(10))),
				"an ordinary attachment replaces the embed viewer"
			);
			view.download.view_embed(message.id, &media);
			banner_frame(&ctx, &mut view, &mut state, vec![], false);
			assert!(
				view.embed_viewing.is_some() && view.viewing.is_none(),
				"an embed replaces the ordinary attachment viewer"
			);
			if reason == 1 {
				message.embeds_suppressed = true;
			} else {
				message.content.push_str(" edited");
			}
			state.timeline.insert(message.clone(), true, false).unwrap();
			banner_frame(&ctx, &mut view, &mut state, vec![], false);
			assert!(view.embed_viewing.is_none());
			if reason == 2 {
				view.download.view_embed(message.id, &media);
				banner_frame(&ctx, &mut view, &mut state, vec![], false);
				assert!(
					view.embed_viewing.is_none(),
					"editing invalidates previous spoiler permission"
				);
			} else if reason == 0 {
				view.download.view_embed(message.id, &media);
				banner_frame(&ctx, &mut view, &mut state, vec![], false);
				assert!(view.embed_viewing.is_some());
				let mut channel = state.channels[0].clone();
				channel.id = Id(21);
				state.channels.push(channel);
				state.selected = Some(Id(21));
				banner_frame(&ctx, &mut view, &mut state, vec![], false);
				assert!(
					view.embed_viewing.is_none(),
					"channel navigation resets the embed viewer"
				);
			}
		}
	}

	#[test]
	fn compact_message_text_baselines_align() {
		fn collect(shape: &egui::Shape, labels: &mut Vec<(String, egui::Pos2)>) {
			match shape {
				egui::Shape::Text(text) => {
					if let Some(row) = text.galley.rows.first()
						&& let Some(glyph) = row.glyphs.first()
					{
						labels.push((
							text.galley.job.text.clone(),
							text.pos + row.pos.to_vec2() + glyph.pos.to_vec2(),
						));
					}
				}
				egui::Shape::Vec(shapes) => shapes.iter().for_each(|shape| collect(shape, labels)),
				_ => {}
			}
		}
		for size in [10.0, 15.0, 20.0] {
			for width in [440.0, 900.0] {
				let ctx = egui::Context::default();
				crate::fonts::install(&ctx);
				crate::design::apply(&ctx);
				ctx.all_styles_mut(|style| {
					style
						.text_styles
						.get_mut(&egui::TextStyle::Body)
						.unwrap()
						.size = size
				});
				let mut state = loading_unread_channel(false);
				state.freshness = model::Freshness::Fresh;
				state.history_pending = false;
				let mut message = text_message(20);
				message.author.name = "Received author".into();
				message.author.kind = model::AccountKind::VerifiedBot;
				message.content =
					"Received body with enough words to wrap in the narrow viewport".into();
				state.user = Some(model::User {
					name: "Pending author".into(),
					..message.author.clone()
				});
				state.timeline.insert(message, false, false).unwrap();
				let mut second = text_message(21);
				second.author.name = "Second author".into();
				second.content = "Second body".into();
				state.timeline.insert(second, false, false).unwrap();
				state.pending.push(client_core::Pending {
					channel: Id(20),
					nonce: "compact-baseline".into(),
					content: "Pending body".into(),
					attachments: vec![],
					sticker: None,
					delivery: model::Delivery::Sending,
					confirmed: None,
				});
				let mut view = TimelineView {
					compact_messages: true,
					..Default::default()
				};
				let mut labels = vec![];
				for _ in 0..6 {
					let output = ctx.run_ui(
						egui::RawInput {
							screen_rect: Some(egui::Rect::from_min_size(
								egui::Pos2::ZERO,
								egui::vec2(width, 600.0),
							)),
							..Default::default()
						},
						|ui| {
							view.show(
								ui,
								&mut state,
								&mut None,
								&mut None,
								(
									&mut crate::avatars::Avatars::default(),
									&mut crate::profiles::ProfileSession::default(),
								),
								None,
							);
						},
					);
					labels.clear();
					for shape in &output.shapes {
						collect(&shape.shape, &mut labels);
					}
					output.drop_without_applying_deltas();
				}
				for (author, body) in [
					("Received author", "Received body"),
					("Second author", "Second body"),
					("Pending author", "Pending body"),
				] {
					let author = labels.iter().find(|(text, _)| text == author).unwrap().1;
					let body = labels
						.iter()
						.find(|(text, _)| text.starts_with(body))
						.unwrap()
						.1;
					assert!(
						(author.y - body.y).abs() <= 1.0 && author.x < body.x,
						"author {author:?}, body {body:?}; font {size}, width {width}"
					);
				}
				let first = labels
					.iter()
					.find(|(text, _)| text == "Received author")
					.unwrap()
					.1;
				let second = labels
					.iter()
					.find(|(text, _)| text == "Second author")
					.unwrap()
					.1;
				let height = ctx.fonts_mut(|fonts| {
					fonts.row_height(&egui::FontId::new(15.5, crate::design::medium_family(&ctx)))
				});
				assert!(
					second.y - first.y >= height,
					"consecutive authors must not overlap: {first:?}, {second:?}"
				);
				let time = labels
					.iter()
					.find(|(text, _)| text.len() == 5 && text.as_bytes()[2] == b':')
					.unwrap()
					.1;
				let body = labels
					.iter()
					.find(|(text, _)| text.starts_with("Received body"))
					.unwrap()
					.1;
				assert!(
					(time.y - body.y).abs() <= 1.0,
					"time {time:?}, body {body:?}; font {size}"
				);
			}
		}
	}

	#[test]
	fn compact_messages_place_authors_beside_every_body_and_reduce_row_height() {
		let mut heights = Vec::new();
		for compact in [false, true] {
			let ctx = egui::Context::default();
			crate::design::apply(&ctx);
			let mut state = loading_unread_channel(false);
			state.freshness = model::Freshness::Fresh;
			state.history_pending = false;
			for id in [20, 21] {
				let mut message = text_message(id);
				message.author.name = "Synthetic compact speaker".into();
				message.content = format!("Compact body {id}");
				state.timeline.insert(message, false, false).unwrap();
			}
			let mut view = TimelineView {
				compact_messages: compact,
				..Default::default()
			};
			for _ in 0..5 {
				banner_frame(&ctx, &mut view, &mut state, vec![], false);
			}
			let labels = banner_frame_bounds(&ctx, &mut view, &mut state, vec![], false, true);
			assert_eq!(
				view.compact_messages, compact,
				"channel initialization preserves density"
			);
			if compact {
				let names: Vec<_> = labels
					.iter()
					.filter(|(text, _)| text == "Synthetic compact speaker")
					.map(|(_, rect)| rect)
					.collect();
				assert_eq!(names.len(), 2, "every compact row retains its author");
				for id in [20, 21] {
					let (_, body) = labels
						.iter()
						.find(|(text, _)| text == &format!("Compact body {id}"))
						.unwrap();
					assert!(
						names
							.iter()
							.any(|name| (name.top() - body.top()).abs() < 4.0
								&& name.right() < body.left()),
						"authors {names:?}, body {body:?}"
					);
				}
			}
			heights.push(view.rows.iter().map(|(_, height)| height).sum::<f32>());
		}
		assert!(
			heights[1] < heights[0],
			"compact {heights:?} should use less vertical space"
		);
	}

	#[test]
	fn compact_authors_remain_beside_leading_quote_and_code_blocks() {
		for (content, body, first_row) in [
			(
				"> Quoted compact body\n> Continued quote\n\nAfter quote",
				"Quoted compact body",
				"Quoted compact body",
			),
			(
				"```rust\nleading_code_body();\n```\nAfter code",
				"leading_code_body();",
				"Rust",
			),
			(
				"> ```rust\n> nested_code_body();\n> ```",
				"nested_code_body();",
				"Rust",
			),
		] {
			for width in [900.0, 440.0] {
				let ctx = egui::Context::default();
				crate::design::apply(&ctx);
				let mut state = loading_unread_channel(false);
				state.freshness = model::Freshness::Fresh;
				state.history_pending = false;
				let mut message = text_message(20);
				message.author.name = "LongSyntheticAuthorDisplayName".into();
				message.content = content.into();
				state.timeline.insert(message, false, false).unwrap();
				let mut view = TimelineView {
					compact_messages: true,
					..Default::default()
				};
				for _ in 0..5 {
					banner_frame_bounds_width(
						&ctx,
						&mut view,
						&mut state,
						vec![],
						false,
						true,
						width,
					);
				}
				let labels = banner_frame_bounds_width(
					&ctx,
					&mut view,
					&mut state,
					vec![],
					false,
					true,
					width,
				);
				let author = labels
					.iter()
					.find(|(text, _)| text == "LongSyntheticAuthorDisplayName")
					.unwrap()
					.1;
				let block = labels
					.iter()
					.find(|(text, _)| text.contains(body))
					.unwrap()
					.1;
				assert!(
					author.right() < block.left(),
					"author {author:?} must stay beside {block:?}: {content}"
				);
				assert!(author.width() <= 161.0, "bounded author: {author:?}");
				assert!(
					block.right() <= width + 1.0,
					"block fits narrow row: {block:?}, width {width}"
				);
				let first_row = labels
					.iter()
					.find(|(text, _)| text.contains(first_row))
					.unwrap()
					.1;
				assert!(
					(author.top() - first_row.top()).abs() < 32.0,
					"author {author:?} remains beside the first block row {first_row:?}: {content}"
				);
			}
		}
	}

	#[test]
	fn unread_banner_is_visible_as_soon_as_an_unread_channel_is_joined() {
		let ctx = egui::Context::default();
		for cached_reply in [true, false] {
			let mut state = loading_unread_channel(cached_reply);
			let mut view = TimelineView::default();
			let labels = banner_frame(&ctx, &mut view, &mut state, vec![], false);
			assert!(
				labels.iter().any(|(text, _)| text == "Mark as read"),
				"cached reply {cached_reply} while history is still loading showed {labels:?}"
			);
			assert!(
				view.mark_read.is_none(),
				"the loading frame must not acknowledge the channel"
			);
		}
	}

	#[test]
	fn complete_short_channels_never_flash_banners_but_tall_unread_content_is_preserved() {
		for (count, tall, latest) in [
			(0, false, 20),
			(1, false, 20),
			(1, false, 21),
			(1, true, 20),
		] {
			let mut state = test_support::demo_state();
			state.timeline.clear();
			state.selected = Some(Id(20));
			state.auth = client_core::auth::AuthState::Authenticated;
			state.gateway_connected = true;
			state.freshness = model::Freshness::Fresh;
			state.history_pending = false;
			state.history_targeted = false;
			state.history_before = None;
			state.history_after = None;
			state.older_exhausted = true;
			state.channels = vec![model::Channel {
				id: Id(20),
				guild: None,
				parent_id: None,
				position: 0,
				name: "Synthetic complete channel".into(),
				kind: 1,
				recipients: vec![],
				member_list_id: None,
				tags: None,
				message_count: None,
				icon: None,
				// Empty/short history can retain stale service latest metadata.
				last_message: Some(Id(latest)),
			}];
			state
				.apply_read_state(client_core::read_state::Event::Snapshot {
					entries: Some(vec![(Id(20), None, 0)]),
					version: None,
					partial: false,
				})
				.unwrap();
			if count == 1 {
				let mut message = text_message(20);
				if tall {
					message.content = "Synthetic tall unread row\n\n".repeat(250);
				}
				state.timeline.insert(message, false, false).unwrap();
			}
			let ctx = egui::Context::default();
			let mut view = TimelineView::default();
			for _ in 0..5 {
				let labels = banner_frame(&ctx, &mut view, &mut state, vec![], false);
				if tall {
					assert!(view.following && view.hold_read_ack && view.mark_read.is_none());
					assert!(
						view.scroll_offset > 400.0,
						"unread join opened at offset {}",
						view.scroll_offset
					);
					assert!(
						labels.iter().any(|(text, _)| text == "Mark as read"),
						"unread join at the bottom hid the banner: {labels:?}"
					);
					continue;
				}
				if count == 0 {
					for forbidden in [
						"Mark as read",
						"Viewing older messages",
						"Jump to unread",
						"New messages below",
						"Jump to present",
					] {
						assert!(
							!labels.iter().any(|(text, _)| text == forbidden),
							"{count} messages unexpectedly showed {forbidden}"
						);
					}
					assert!(!view.hold_read_ack && view.following);
				} else {
					assert!(
						labels.iter().any(|(text, _)| text == "Mark as read"),
						"unacked short join hid the banner: {labels:?}"
					);
					assert!(view.following && view.hold_read_ack && view.mark_read.is_none());
				}
			}
			assert_eq!(view.mark_read, (count == 0).then_some(Id(latest)));
			if tall {
				// Scrolling to the live edge of tall unread content acknowledges it and resolves
				// both banners, even when the service latest ID names a deleted message.
				state.channels[0].last_message = Some(Id(21));
				view.mark_read = None;
				view.anchor = Some((Id(20), f32::MAX));
				view.revision = u64::MAX;
				banner_frame(&ctx, &mut view, &mut state, vec![], false);
				let labels = banner_frame(
					&ctx,
					&mut view,
					&mut state,
					vec![
						egui::Event::PointerMoved(egui::pos2(450.0, 300.0)),
						egui::Event::MouseWheel {
							unit: egui::MouseWheelUnit::Point,
							delta: egui::vec2(0.0, -600.0),
							modifiers: egui::Modifiers::NONE,
							phase: egui::TouchPhase::Move,
							source: egui::MouseWheelSource::Unknown,
						},
					],
					false,
				);
				assert!(view.following && !view.target_browsing);
				assert_eq!(view.mark_read.take(), Some(Id(21)));
				for forbidden in [
					"Mark as read",
					"Next messages",
					"New messages below",
					"Jump to present",
				] {
					assert!(
						!labels.iter().any(|(text, _)| text == forbidden),
						"tall stale channel still showed {forbidden}"
					);
				}
			}
			if count == 1 && !tall {
				// A short channel never scrolls, so the banner offers to acknowledge it directly.
				let labels = banner_frame(&ctx, &mut view, &mut state, vec![], false);
				let pos = labels
					.iter()
					.find(|(text, _)| text == "Mark as read")
					.map(|(_, rect)| rect.center())
					.expect("the unread banner offers Mark as read");
				for pressed in [true, false] {
					banner_frame(
						&ctx,
						&mut view,
						&mut state,
						vec![
							egui::Event::PointerMoved(pos),
							egui::Event::PointerButton {
								pos,
								button: egui::PointerButton::Primary,
								pressed,
								modifiers: egui::Modifiers::NONE,
							},
						],
						false,
					);
				}
				assert_eq!(view.mark_channel_read.take(), Some(Id(20)));
				let labels = banner_frame(&ctx, &mut view, &mut state, vec![], false);
				assert!(
					!labels.iter().any(|(text, _)| text == "Mark as read"),
					"Mark as read left the banner up: {labels:?}"
				);
			}
			if count == 1 && !tall && latest == 20 {
				// A read snapshot arriving after a local reply jump must not resume reading.
				state.read_state.reset();
				let mut view = TimelineView::default();
				state.search_target = Some(Id(20));
				banner_frame(&ctx, &mut view, &mut state, vec![], false);
				state
					.apply_read_state(client_core::read_state::Event::Snapshot {
						entries: Some(vec![(Id(20), None, 0)]),
						version: None,
						partial: false,
					})
					.unwrap();
				banner_frame(&ctx, &mut view, &mut state, vec![], false);
				assert!(view.target_browsing && view.mark_read.is_none());
			}
		}
	}

	#[test]
	fn downward_wheel_at_pinned_target_loads_next_page() {
		let mut state = test_support::demo_state();
		state.timeline.clear();
		state.selected = Some(Id(20));
		state.freshness = model::Freshness::Fresh;
		state.history_pending = false;
		state.history_targeted = true;
		state.history_before = Some(Id(51));
		state
			.channels
			.iter_mut()
			.find(|channel| channel.id == Id(20))
			.unwrap()
			.last_message = Some(Id(100));
		for id in 1..=50 {
			state
				.timeline
				.insert(text_message(id), false, false)
				.unwrap();
		}
		state.search_target = Some(Id(50));
		state.revision += 1;
		assert!(state.can_load_newer());

		let ctx = egui::Context::default();
		crate::design::apply(&ctx);
		let mut view = TimelineView::default();
		for _ in 0..80 {
			banner_frame(&ctx, &mut view, &mut state, vec![], false);
			if view.reveal_scroll.is_none() && view.highlighted.is_some() {
				break;
			}
		}
		assert!(!view.at_current_latest);

		banner_frame(
			&ctx,
			&mut view,
			&mut state,
			vec![
				egui::Event::PointerMoved(egui::pos2(450.0, 300.0)),
				egui::Event::MouseWheel {
					unit: egui::MouseWheelUnit::Point,
					delta: egui::vec2(0.0, -600.0),
					modifiers: egui::Modifiers::NONE,
					phase: egui::TouchPhase::Move,
					source: egui::MouseWheelSource::Unknown,
				},
			],
			false,
		);

		assert!(view.load_newer);
		assert!(view.target_browsing);
	}

	#[test]
	fn browsing_banner_waits_several_screens_and_click_survives_widget_order_changes() {
		let ctx = egui::Context::default();
		let mut state = test_support::demo_state();
		state.read_state.reset();
		state.timeline.clear();
		state.selected = Some(Id(20));
		state.history_targeted = false;
		state.history_before = None;
		state.history_after = None;
		for id in 1..=200 {
			state
				.timeline
				.insert(text_message(id), false, false)
				.unwrap();
		}
		state.revision += 1;
		let mut view = TimelineView::default();
		for _ in 0..5 {
			banner_frame(&ctx, &mut view, &mut state, vec![], false);
		}
		// Several screens of scrollback pass before the control appears at all.
		for (distance, expected) in [(1_200.0, false), (3_000.0, false), (4_200.0, true)] {
			for _ in 0..5 {
				let offset =
					view.rows.iter().map(|(_, height)| height).sum::<f32>() - 600.0 - distance;
				let (index, _, top) = visible_range(&view.rows, offset, offset);
				view.following = false;
				view.jump = false;
				view.anchor = Some((view.rows[index].0, offset - top));
				view.revision = u64::MAX;
				banner_frame(&ctx, &mut view, &mut state, vec![], false);
			}
			assert_eq!(view.present_control.is_some(), expected);
		}
		banner_frame(&ctx, &mut view, &mut state, vec![], false);
		let pos = view.present_control.unwrap().center();
		for pressed in [true, false] {
			banner_frame(
				&ctx,
				&mut view,
				&mut state,
				vec![
					egui::Event::PointerMoved(pos),
					egui::Event::PointerButton {
						pos,
						button: egui::PointerButton::Primary,
						pressed,
						modifiers: egui::Modifiers::NONE,
					},
				],
				!pressed,
			);
		}
		assert!(
			view.present_scroll.is_some(),
			"Jump to present lost its click when sibling widget IDs changed"
		);
		// The glide back to the live edge resumes following without a teleport.
		for _ in 0..30 {
			banner_frame(&ctx, &mut view, &mut state, vec![], false);
		}
		assert!(view.present_scroll.is_none() && view.following);
	}
	#[test]
	fn pending_rows_share_scroll_and_only_measure_near_viewport() {
		let ctx = egui::Context::default();
		let mut state = State {
			demo: true,
			selected: Some(Id(20)),
			..Default::default()
		};
		state.pending = (0..64)
			.map(|i| client_core::Pending {
				sticker: None,
				channel: Id(20),
				nonce: i.to_string(),
				content: format!("Pending message {i}"),
				attachments: vec![],
				delivery: model::Delivery::Sending,
				confirmed: None,
			})
			.collect();
		let mut view = TimelineView {
			channel: state.selected,
			..Default::default()
		};
		let render = |view: &mut TimelineView, state: &mut State| {
			ctx.run_ui(
				egui::RawInput {
					screen_rect: Some(egui::Rect::from_min_size(
						egui::Pos2::ZERO,
						egui::vec2(400.0, 300.0),
					)),
					..Default::default()
				},
				|ui| {
					view.show(
						ui,
						state,
						&mut None,
						&mut None,
						(
							&mut crate::avatars::Avatars::default(),
							&mut crate::profiles::ProfileSession::default(),
						),
						None,
					);
				},
			)
			.drop_without_applying_deltas();
		};
		for _ in 0..3 {
			render(&mut view, &mut state);
		}
		assert_ne!(view.pending_heights["0"], 100.0);
		let measured_at_top: Vec<_> = view
			.pending_heights
			.iter()
			.filter(|(_, height)| **height != 100.0)
			.map(|(nonce, _)| nonce.parse::<usize>().unwrap())
			.collect();
		let mut top = 0.0;
		for index in 0..64 {
			if measured_at_top.contains(&index) {
				assert!(
					top <= 300.0 + 100.0,
					"only rows near the viewport should be measured"
				);
			}
			top += view.pending_heights[&index.to_string()];
		}
		assert_eq!(view.pending_heights["63"], 100.0);
		view.follow_latest(&state);
		for _ in 0..5 {
			render(&mut view, &mut state);
		}
		assert_ne!(view.pending_heights["63"], 100.0);
		assert!(view.following);
		let mut bottom = 0.0;
		for index in (0..64).rev() {
			let height = view.pending_heights[&index.to_string()];
			if height != 100.0 && !measured_at_top.contains(&index) {
				assert!(
					bottom <= 300.0 + 100.0,
					"jumping should only measure rows near the bottom viewport"
				);
			}
			bottom += height;
		}
		assert_eq!(view.pending_heights["32"], 100.0);
		state.pending.clear();
		render(&mut view, &mut state);
		assert!(view.pending_heights.is_empty());
	}

	fn text_message(id: u64) -> Message {
		Message {
			poll: None,
			sticker_items: vec![],
			id: Id(id),
			channel: Id(20),
			author: model::User {
				id: Id(2),
				name: "Robin".into(),
				avatar: None,
				webhook: false,
				kind: Default::default(),
				discriminator: 0,
				primary_guild: None,
			},
			content: "Synthetic text with enough words to wrap in a narrow viewport.".into(),
			edited: false,
			edited_at: None,
			revision: 0,
			nonce: None,
			reply_to: None,
			kind: 0,
			reply_deleted: false,
			interaction: None,
			forwarded: false,
			unsupported: false,
			extra_content: Default::default(),
			components: vec![],
			application_id: None,
			ephemeral: false,
			flags: 0,
			embeds: vec![],
			attachments: vec![],
			author_nick: None,
			author_roles: vec![],
			mention_roles: vec![],
			mention_everyone: false,
			suppress_notifications: false,
			mentions: vec![],
			reactions: Some(vec![]),
			embeds_suppressed: false,
		}
	}

	#[test]
	fn forwarded_audio_keeps_sender_label_and_player_in_narrow_and_wide_rows() {
		fn text(shape: &egui::Shape, out: &mut Vec<String>) {
			match shape {
				egui::Shape::Text(t) => out.push(t.galley.job.text.clone()),
				egui::Shape::Vec(shapes) => {
					for shape in shapes {
						text(shape, out);
					}
				}
				_ => {}
			}
		}
		for width in [320.0, 960.0] {
			let ctx = egui::Context::default();
			let mut state = State {
				selected: Some(Id(20)),
				demo: true,
				..Default::default()
			};
			let mut message = text_message(1);
			let key = layout_key(&message);
			message.forwarded = true;
			assert_ne!(key, layout_key(&message));
			message.content = "Forwarded caption".into();
			message.attachments = vec![model::Attachment {
				id: Id(3),
				filename: "Synthetic.mp3".into(),
				size: 4_000_000,
				description: None,
				content_type: Some("audio/mpeg".into()),
				spoiler: false,
				duration_ms: Some(168_000),
				waveform: vec![],
				media: model::EmbedMedia {
					url: Some("https://cdn.discordapp.com/attachments/20/3/synthetic.mp3".into()),
					..Default::default()
				},
			}];
			state.timeline.insert(message, false, false).unwrap();
			let mut view = TimelineView::default();
			let mut avatars = crate::avatars::Avatars::default();
			let mut painted = vec![];
			for _ in 0..4 {
				painted.clear();
				let output = ctx.run_ui(
					egui::RawInput {
						screen_rect: Some(egui::Rect::from_min_size(
							egui::Pos2::ZERO,
							egui::vec2(width, 700.0),
						)),
						..Default::default()
					},
					|ui| {
						view.show(
							ui,
							&mut state,
							&mut None,
							&mut None,
							(
								&mut avatars,
								&mut crate::profiles::ProfileSession::default(),
							),
							None,
						);
						assert!(
							ui.min_rect().width() <= width,
							"row width {} exceeds {width}",
							ui.min_rect().width()
						);
					},
				);
				for shape in &output.shapes {
					text(&shape.shape, &mut painted);
				}
				output.drop_without_applying_deltas();
			}
			for label in [
				"Robin",
				"\u{21aa} Forwarded",
				"Forwarded caption",
				"Synthetic.mp3",
			] {
				assert!(
					painted.iter().any(|text| text == label),
					"missing {label}: {painted:?}"
				);
			}
			assert!(
				!painted
					.iter()
					.any(|text| text.contains("Unsupported message"))
			);
		}
	}
	#[test]
	fn message_menu_allows_authorized_delete_without_exposing_other_authors_edit() {
		fn collect(shape: &egui::Shape, labels: &mut Vec<(String, egui::Rect)>) {
			match shape {
				egui::Shape::Text(text) => {
					labels.push((text.galley.job.text.clone(), text.visual_bounding_rect()))
				}
				egui::Shape::Vec(shapes) => {
					for shape in shapes {
						collect(shape, labels);
					}
				}
				_ => {}
			}
		}
		for (own, can_delete) in [(false, true), (false, false), (true, false)] {
			let ctx = egui::Context::default();
			let message = text_message(1);
			let mut thread_request = None;
			let mut editing = None;
			let mut edit_started = false;
			let mut deleting = None;
			let mut reply = None;
			let mut frame = |events: Vec<egui::Event>| {
				let output = ctx.run_ui(
					egui::RawInput {
						focused: true,
						events,
						screen_rect: Some(egui::Rect::from_min_size(
							egui::Pos2::ZERO,
							egui::vec2(320.0, 400.0),
						)),
						..Default::default()
					},
					|ui| {
						let menu = action_button(ui, crate::icons::Icon::More, "More");
						message_actions(
							egui::Popup::menu(&menu),
							(&message, None, &[], &mut None),
							(own, true, true, can_delete),
							(None, None, &mut reply),
							(&mut editing, &mut edit_started),
							&mut deleting,
							(false, false, &mut None),
							(own, &mut thread_request),
							(false, &mut None),
							None,
						)
					},
				);
				assert!(
					output
						.platform_output
						.commands
						.iter()
						.all(|command| matches!(
							command,
							egui::OutputCommand::TextSelectionSettled(_)
						))
				);
				let mut labels = vec![];
				for shape in &output.shapes {
					collect(&shape.shape, &mut labels);
				}
				output.drop_without_applying_deltas();
				labels
			};
			for _ in 0..2 {
				frame(vec![]);
			}
			// The real More button is keyboard reachable and opens the native egui menu.
			for key in [egui::Key::Tab, egui::Key::Enter] {
				frame(vec![egui::Event::Key {
					key,
					physical_key: None,
					pressed: true,
					repeat: false,
					modifiers: egui::Modifiers::NONE,
				}]);
			}
			frame(vec![]);
			let labels = frame(vec![]);
			assert!(labels.iter().any(|(label, _)| label == "Copy message"));
			assert_eq!(
				labels
					.iter()
					.any(|(label, _)| label.starts_with("Create Thread")),
				own,
				"Create Thread appears only where a thread may be started"
			);
			assert_eq!(labels.iter().any(|(label, _)| label == "Edit message"), own);
			let delete = labels
				.iter()
				.find(|(label, _)| label.starts_with("Delete message"));
			assert_eq!(delete.is_some(), own || can_delete);
			let action = if own {
				labels.iter().find(|(label, _)| label == "Edit message")
			} else {
				delete
			};
			if let Some((_, rect)) = action {
				let pos = rect.center();
				for pressed in [true, false] {
					frame(vec![
						egui::Event::PointerMoved(pos),
						egui::Event::PointerButton {
							pos,
							button: egui::PointerButton::Primary,
							pressed,
							modifiers: egui::Modifiers::NONE,
						},
					]);
				}
			}
			assert!(reply.is_none());
			assert_eq!(
				deleting,
				(!own && can_delete).then_some((message.channel, message.id))
			);
			if own {
				assert_eq!(
					editing,
					Some((message.channel, message.id, message.content.clone()))
				);
				assert!(edit_started);
			} else {
				assert!(editing.is_none() && !edit_started);
			}
		}
	}

	#[test]
	fn inline_reveals_are_independent_of_media_and_reset_on_edit_and_navigation() {
		fn collect(shape: &egui::Shape, labels: &mut Vec<(String, egui::Rect)>) {
			match shape {
				egui::Shape::Text(text) => {
					labels.push((text.galley.job.text.clone(), text.visual_bounding_rect()))
				}
				egui::Shape::Vec(shapes) => {
					for shape in shapes {
						collect(shape, labels);
					}
				}
				_ => {}
			}
		}
		for width in [260.0, 800.0] {
			let ctx = egui::Context::default();
			ctx.set_visuals(if width < 300.0 {
				egui::Visuals::light()
			} else {
				egui::Visuals::dark()
			});
			let mut message = text_message(1);
			message.content =
				"Public before ||secret one|| middle ||secret two|| after `||code literal||`"
					.into();
			message.embeds = vec![model::Embed {
				title: Some("Visible card".into()),
				..Default::default()
			}];
			let mut state = State {
				demo: true,
				selected: Some(message.channel),
				..Default::default()
			};
			state
				.timeline
				.insert(message.clone(), false, false)
				.unwrap();
			let mut view = TimelineView::default();
			let mut images = crate::avatars::Avatars::default();
			let mut render = |view: &mut TimelineView, state: &mut State, events| {
				let output = ctx.run_ui(
					egui::RawInput {
						focused: true,
						events,
						screen_rect: Some(egui::Rect::from_min_size(
							egui::Pos2::ZERO,
							egui::vec2(width, 700.0),
						)),
						..Default::default()
					},
					|ui| {
						view.show(
							ui,
							state,
							&mut None,
							&mut None,
							(&mut images, &mut crate::profiles::ProfileSession::default()),
							None,
						)
					},
				);
				assert!(
					output
						.platform_output
						.commands
						.iter()
						.all(|command| matches!(
							command,
							egui::OutputCommand::TextSelectionSettled(_)
						))
				);
				let mut labels = vec![];
				for shape in &output.shapes {
					collect(&shape.shape, &mut labels);
				}
				output.drop_without_applying_deltas();
				assert!(view.opening.is_none() && view.channel_reference.is_none());
				labels
			};
			for _ in 0..3 {
				render(&mut view, &mut state, vec![]);
			}
			let labels = render(&mut view, &mut state, vec![]);
			let visible: String = labels.iter().map(|(text, _)| text.as_str()).collect();
			assert!(visible.contains("Public before") && visible.contains("Visible card"));
			assert!(visible.contains("||code literal||"));
			assert!(!visible.contains("secret one") && !visible.contains("secret two"));
			assert_eq!(
				labels
					.iter()
					.filter(|(text, _)| text == "Reveal spoiler")
					.count(),
				2
			);
			let click = |label: &str, labels: &[(String, egui::Rect)]| {
				let pos = labels
					.iter()
					.find(|(text, _)| text == label)
					.unwrap()
					.1
					.center();
				[true, false].map(|pressed| {
					vec![
						egui::Event::PointerMoved(pos),
						egui::Event::PointerButton {
							pos,
							button: egui::PointerButton::Primary,
							pressed,
							modifiers: egui::Modifiers::NONE,
						},
					]
				})
			};
			for events in click("Reveal spoiler", &labels) {
				render(&mut view, &mut state, events);
			}
			let labels = render(&mut view, &mut state, vec![]);
			let visible: String = labels.iter().map(|(text, _)| text.as_str()).collect();
			assert!(visible.contains("secret one") && !visible.contains("secret two"));
			assert_eq!(view.revealed[&message.id].text, 1);
			assert!(!view.revealed[&message.id].media);
			for events in click("Hide spoilers", &labels) {
				render(&mut view, &mut state, events);
			}
			render(&mut view, &mut state, vec![]);
			assert!(view.revealed.is_empty());

			// A text reveal cannot grant access to a separately concealed card.
			message.embeds[0].title = Some("||hidden card||".into());
			state.timeline.insert(message.clone(), true, false).unwrap();
			state.revision += 1;
			let labels = render(&mut view, &mut state, vec![]);
			for events in click("Reveal spoiler", &labels) {
				render(&mut view, &mut state, events);
			}
			let labels = render(&mut view, &mut state, vec![]);
			assert!(
				labels
					.iter()
					.any(|(text, _)| text == "Reveal spoiler media")
			);
			assert!(!labels.iter().any(|(text, _)| text.contains("hidden card")));
			for events in click("Reveal spoiler media", &labels) {
				render(&mut view, &mut state, events);
			}
			let labels = render(&mut view, &mut state, vec![]);
			assert!(labels.iter().any(|(text, _)| text.contains("hidden card")));
			assert!(view.revealed[&message.id].media);

			message.content = "Public changed ||new secret||".into();
			state.timeline.insert(message.clone(), true, false).unwrap();
			state.revision += 1;
			let labels = render(&mut view, &mut state, vec![]);
			assert!(
				!labels
					.iter()
					.any(|(text, _)| text.contains("new secret") || text.contains("hidden card"))
			);
			assert!(view.revealed.is_empty());
			let current = labels
				.iter()
				.rfind(|(text, _)| text == "Reveal spoiler")
				.unwrap()
				.1
				.center();
			for pressed in [true, false] {
				render(
					&mut view,
					&mut state,
					vec![
						egui::Event::PointerMoved(current),
						egui::Event::PointerButton {
							pos: current,
							button: egui::PointerButton::Primary,
							pressed,
							modifiers: egui::Modifiers::NONE,
						},
					],
				);
			}
			let labels = render(&mut view, &mut state, vec![]);
			let visible: String = labels.iter().map(|(text, _)| text.as_str()).collect();
			assert!(visible.contains("new secret"));
			assert_eq!(view.revealed[&message.id].text, 1);
			state.selected = Some(Id(30));
			render(&mut view, &mut state, vec![]);
			assert!(view.revealed.is_empty());
		}
	}
	#[test]
	fn thread_starter_renders_above_replies_once_history_is_exhausted() {
		fn text(shape: &egui::Shape, out: &mut Vec<String>) {
			match shape {
				egui::Shape::Text(t) => out.push(t.galley.job.text.clone()),
				egui::Shape::Vec(shapes) => shapes.iter().for_each(|s| text(s, out)),
				_ => {}
			}
		}
		let ctx = egui::Context::default();
		crate::design::apply(&ctx);
		let mut state = test_support::chat_demo_state();
		state
			.permissions
			.replace(test_support::permission_snapshot(&state))
			.unwrap();
		let (thread, parent) = state
			.channels
			.iter()
			.find(|c| {
				matches!(c.kind, 10..=12)
					&& c.parent_id
						.and_then(|id| state.channel(id))
						.is_some_and(|p| matches!(p.kind, 0 | 5))
			})
			.map(|c| (c.id, c.parent_id.unwrap()))
			.expect("fixture thread under a text channel");
		state.select(thread);
		for id in [thread.0 + 10, thread.0 + 20] {
			state
				.timeline
				.insert(test_support::message(id, thread), false, false)
				.unwrap();
		}
		state.history_pending = false;
		state.freshness = model::Freshness::Fresh;
		state.older_exhausted = true;
		let Some(client_core::Command::ThreadStarter { request, .. }) =
			state.request_thread_starter()
		else {
			panic!("thread requests its starter");
		};
		let mut starter = test_support::message(thread.0, parent);
		starter.content = "The message that started it all".into();
		state.apply(client_core::Envelope {
			generation: state.generation,
			event: client_core::Event::ThreadStarter {
				thread,
				request,
				result: Ok(starter),
			},
		});
		let mut view = TimelineView::default();
		let mut avatars = crate::avatars::Avatars::default();
		let mut painted = vec![];
		for _ in 0..4 {
			painted.clear();
			let output = ctx.run_ui(
				egui::RawInput {
					screen_rect: Some(egui::Rect::from_min_size(
						egui::Pos2::ZERO,
						egui::vec2(900.0, 1400.0),
					)),
					..Default::default()
				},
				|ui| {
					view.show(
						ui,
						&mut state,
						&mut None,
						&mut None,
						(
							&mut avatars,
							&mut crate::profiles::ProfileSession::default(),
						),
						None,
					);
				},
			);
			for shape in &output.shapes {
				text(&shape.shape, &mut painted);
			}
			output.drop_without_applying_deltas();
		}
		assert_eq!(view.rows.first().map(|(id, _)| *id), Some(thread));
		for label in [
			"The message that started it all",
			"Thread started from this message",
		] {
			assert!(
				painted.iter().any(|t| t == label),
				"missing {label}: {painted:?}"
			);
		}
	}
	#[test]
	fn system_events_render_wrap_and_keep_unknown_fallbacks() {
		fn text(shape: &egui::Shape, out: &mut Vec<String>) {
			match shape {
				egui::Shape::Text(t) => out.push(t.galley.job.text.clone()),
				egui::Shape::Vec(shapes) => shapes.iter().for_each(|s| text(s, out)),
				_ => {}
			}
		}
		for (width, dark) in [(900.0, true), (280.0, false)] {
			let ctx = egui::Context::default();
			crate::design::apply(&ctx);
			ctx.set_visuals(if dark {
				egui::Visuals::dark()
			} else {
				egui::Visuals::light()
			});
			let mut state = State {
				selected: Some(Id(20)),
				demo: true,
				..Default::default()
			};
			for (id, kind, content) in [
				(1, 7, ""),
				(2, 4, "new channel name"),
				(3, 222, ""),
				(4, 67, ""),
				(5, 59, ""),
				(6, 65, ""),
				(7, 30, ""),
				(8, 55, ""),
				(9, 58, ""),
				(10, 60, ""),
				(11, 61, ""),
				(12, 62, ""),
			] {
				let mut message = text_message(id);
				let old_key = layout_key(&message);
				message.kind = kind;
				assert_ne!(old_key, layout_key(&message));
				message.unsupported = true;
				message.content = content.into();
				assert!(!grouped(Some(&message), &message, None));
				state.timeline.insert(message, false, false).unwrap();
			}
			let mut view = TimelineView::default();
			let mut avatars = crate::avatars::Avatars::default();
			let mut painted = vec![];
			for _ in 0..5 {
				painted.clear();
				let output = ctx.run_ui(
					egui::RawInput {
						screen_rect: Some(egui::Rect::from_min_size(
							egui::Pos2::ZERO,
							egui::vec2(width, 2000.0),
						)),
						..Default::default()
					},
					|ui| {
						view.show(
							ui,
							&mut state,
							&mut None,
							&mut None,
							(
								&mut avatars,
								&mut crate::profiles::ProfileSession::default(),
							),
							None,
						);
						assert!(ui.min_rect().width() <= width, "system rows overflow");
						let colors = crate::design::palette(ui);
						assert_eq!(
							system_icon(67, &colors),
							(crate::icons::Icon::Check, colors.positive)
						);
					},
				);
				for shape in &output.shapes {
					text(&shape.shape, &mut painted);
				}
				output.drop_without_applying_deltas();
			}
			// System rows paint the sentence as styled runs with the member name strong.
			assert!(painted.iter().any(|s| s == "Welcome, "));
			assert!(painted.iter().any(|s| s == "! Joined the server."));
			assert!(painted.iter().any(|s| s == " changed the channel name"));
			assert!(painted.iter().any(|s| s == "new channel name"));
			for description in [
				" accepted your friend request.",
				" timed out ",
				" started a voice hangout.",
				" requested to speak.",
				" upgraded the stream to HD.",
				" deleted a reported message.",
				" kicked ",
				" banned ",
				" resolved a report.",
			] {
				assert!(
					painted.iter().any(|s| s == description),
					"missing {description}"
				);
			}
			// One name per system row; only the unknown type paints an author header.
			assert_eq!(painted.iter().filter(|s| *s == "Robin").count(), 12);
			assert_eq!(
				painted
					.iter()
					.filter(|s| s.contains("Preview unavailable"))
					.count(),
				1
			);
			assert!(
				painted
					.iter()
					.any(|s| s.contains("Unsupported message type 222"))
			);
		}
	}
	#[test]
	fn grouping_respects_dates_replies_unread_and_five_minute_gaps() {
		let mut first = text_message(1);
		let mut next = text_message((60_000 << 22) | 1);
		assert_eq!(timestamp(first.id).date().to_string(), "2015-01-01");
		assert!(grouped(Some(&first), &next, None));
		let original_key = layout_key(&next);
		next.author.kind = model::AccountKind::Bot;
		assert!(!grouped(Some(&first), &next, None));
		assert_ne!(original_key, layout_key(&next));
		next.author.kind = model::AccountKind::Human;
		next.edited = true;
		assert!(grouped(Some(&first), &next, None));
		next.edited = false;
		assert!(!grouped(Some(&first), &next, Some(next.id)));
		let idle = State::default();
		assert_ne!(
			row_key(&next, Some(&first), None, &idle),
			row_key(&next, None, None, &idle)
		);
		next.reply_to = Some(first.id);
		assert!(!grouped(Some(&first), &next, None));
		next.reply_to = None;
		next.id = Id(300_000 << 22);
		assert!(!grouped(Some(&first), &next, None));
		first.id = Id(86_340_000 << 22);
		next.id = Id(86_400_000 << 22);
		assert!(!grouped(Some(&first), &next, None));
		assert_eq!(timestamp(Id(u64::MAX)).year(), 2154);
	}
	#[test]
	fn account_badges_render_in_chat_without_grouping_different_author_kinds() {
		let ctx = egui::Context::default();
		crate::design::apply(&ctx);
		let mut state = State {
			demo: true,
			selected: Some(Id(20)),
			..Default::default()
		};
		for (index, kind, webhook) in [
			(1, model::AccountKind::Bot, false),
			(2, model::AccountKind::App, true),
			(3, model::AccountKind::Human, true),
		] {
			let mut message = text_message(index);
			message.author.kind = kind;
			message.author.webhook = webhook;
			state.timeline.insert(message, false, false).unwrap();
		}
		let mut view = TimelineView::default();
		let mut avatars = crate::avatars::Avatars::default();
		for _ in 0..4 {
			ctx.run_ui(Default::default(), |ui| {
				view.show(
					ui,
					&mut state,
					&mut None,
					&mut None,
					(
						&mut avatars,
						&mut crate::profiles::ProfileSession::default(),
					),
					None,
				);
			})
			.drop_without_applying_deltas();
		}
		let output = ctx.run_ui(Default::default(), |ui| {
			view.show(
				ui,
				&mut state,
				&mut None,
				&mut None,
				(
					&mut avatars,
					&mut crate::profiles::ProfileSession::default(),
				),
				None,
			);
		});
		for expected in ["BOT", "APP", "WEBHOOK"] {
			assert!(
				output.shapes.iter().any(
					|s| matches!(&s.shape, egui::Shape::Text(t) if t.galley.job.text == expected)
				),
				"Missing {expected}"
			);
		}
		output.drop_without_applying_deltas();
	}
	#[test]
	fn short_continuations_use_one_line_and_keep_internal_breaks() {
		for (width, dark) in [(900.0, true), (360.0, false)] {
			let ctx = egui::Context::default();
			crate::design::apply(&ctx);
			ctx.set_theme(if dark {
				egui::Theme::Dark
			} else {
				egui::Theme::Light
			});
			let mut state = test_support::demo_state();
			state.timeline.clear();
			state.read_state.reset();
			for (id, content) in [(1, "First"), (2, "Next"), (3, "One\nTwo")] {
				let mut message = text_message(id);
				message.content = content.into();
				state.timeline.insert(message, false, false).unwrap();
			}
			let mut view = TimelineView::default();
			let mut images = crate::avatars::Avatars::default();
			for _ in 0..5 {
				ctx.run_ui(
					egui::RawInput {
						screen_rect: Some(egui::Rect::from_min_size(
							egui::Pos2::ZERO,
							egui::vec2(width, 600.0),
						)),
						..Default::default()
					},
					|ui| {
						view.show(
							ui,
							&mut state,
							&mut None,
							&mut None,
							(&mut images, &mut crate::profiles::ProfileSession::default()),
							None,
						)
					},
				)
				.drop_without_applying_deltas();
			}
			let short = view.heights[&Id(2)].1;
			assert!(short <= 26.0, "single-line continuation is {short} pt tall");
			assert!(
				view.heights[&Id(3)].1 > short + 8.0,
				"internal newline must remain visible"
			);
		}
	}
	#[test]
	fn unsupported_message_fallback_only_requests_confirmation() {
		fn button(shape: &egui::Shape) -> Option<egui::Rect> {
			match shape {
				egui::Shape::Text(t) if t.galley.job.text == "Open in Discord" => {
					Some(t.galley.rect.translate(t.pos.to_vec2()))
				}
				egui::Shape::Vec(shapes) => shapes.iter().find_map(button),
				_ => None,
			}
		}
		let mut state = test_support::demo_state();
		let channel = state
			.channels
			.iter()
			.find(|c| Some(c.id) == state.selected)
			.unwrap()
			.clone();
		let mut message = text_message(42);
		message.channel = channel.id;
		message.unsupported = true;
		state.timeline.clear();
		state.timeline.insert(message, false, false).unwrap();
		for allowed in [true, false] {
			if !allowed {
				state.channels.clear();
			}
			let ctx = egui::Context::default();
			let mut view = TimelineView::default();
			let mut avatars = crate::avatars::Avatars::default();
			let mut render = |view: &mut TimelineView, events| {
				let output = ctx.run_ui(
					egui::RawInput {
						screen_rect: Some(egui::Rect::from_min_size(
							egui::Pos2::ZERO,
							egui::vec2(360.0, 600.0),
						)),
						events,
						..Default::default()
					},
					|ui| {
						view.show(
							ui,
							&mut state,
							&mut None,
							&mut None,
							(
								&mut avatars,
								&mut crate::profiles::ProfileSession::default(),
							),
							None,
						)
					},
				);
				assert!(
					output
						.platform_output
						.commands
						.iter()
						.all(|command| matches!(
							command,
							egui::OutputCommand::TextSelectionSettled(_)
						))
				);
				let rect = output.shapes.iter().find_map(|s| button(&s.shape));
				output.drop_without_applying_deltas();
				rect
			};
			for _ in 0..3 {
				render(&mut view, vec![]);
			}
			let point = render(&mut view, vec![])
				.expect("Unsupported message has a fallback")
				.center();
			assert!(view.opening.is_none());
			for pressed in [true, false] {
				render(
					&mut view,
					vec![
						egui::Event::PointerMoved(point),
						egui::Event::PointerButton {
							pos: point,
							button: egui::PointerButton::Primary,
							pressed,
							modifiers: egui::Modifiers::NONE,
						},
					],
				);
			}
			assert_eq!(
				view.browser_opening,
				if allowed {
					discord_url(&channel, Some(Id(42)))
				} else {
					None
				}
			);
		}
	}

	#[test]
	fn extra_content_markers_update_layout_and_keep_supported_text() {
		fn collect(shape: &egui::Shape, texts: &mut Vec<(String, egui::Rect)>) {
			match shape {
				egui::Shape::Text(t) => texts.push((
					t.galley.job.text.clone(),
					t.galley.rect.translate(t.pos.to_vec2()),
				)),
				egui::Shape::Vec(shapes) => {
					for shape in shapes {
						collect(shape, texts);
					}
				}
				_ => {}
			}
		}
		let mut state = test_support::demo_state();
		state.read_state.reset();
		let channel = state
			.channels
			.iter()
			.find(|c| Some(c.id) == state.selected)
			.unwrap()
			.clone();
		let mut message = text_message(42);
		message.channel = channel.id;
		message.content = "Supported text remains".into();
		let plain_key = layout_key(&message);
		let ctx = egui::Context::default();
		let mut view = TimelineView::default();
		let mut avatars = crate::avatars::Avatars::default();
		let mut render = |view: &mut TimelineView, state: &mut State, events| {
			let output = ctx.run_ui(
				egui::RawInput {
					screen_rect: Some(egui::Rect::from_min_size(
						egui::Pos2::ZERO,
						egui::vec2(380.0, 650.0),
					)),
					events,
					..Default::default()
				},
				|ui| {
					view.show(
						ui,
						state,
						&mut None,
						&mut None,
						(
							&mut avatars,
							&mut crate::profiles::ProfileSession::default(),
						),
						None,
					)
				},
			);
			assert!(
				output.platform_output.commands.is_empty(),
				"Markers only offer explicit external confirmation"
			);
			let mut texts = vec![];
			for shape in &output.shapes {
				collect(&shape.shape, &mut texts);
			}
			output.drop_without_applying_deltas();
			texts
		};
		let all = model::ExtraContent {
			poll: true,
			sticker_items: true,
			stickers: true,
			components: true,
			components_v2: true,
		};
		let mut full_height = None;
		for extra in [
			all,
			model::ExtraContent {
				sticker_items: true,
				..Default::default()
			},
			model::ExtraContent {
				stickers: true,
				..Default::default()
			},
			model::ExtraContent {
				components_v2: true,
				..Default::default()
			},
			model::ExtraContent::default(),
		] {
			message.extra_content = extra;
			assert_eq!(layout_key(&message) == plain_key, !extra.any());
			let mut previous = message.clone();
			previous.id = Id(41);
			assert_eq!(grouped(Some(&previous), &message, None), !extra.any());
			state.timeline.clear();
			state
				.timeline
				.insert(message.clone(), false, false)
				.unwrap();
			state.revision += 1;
			for _ in 0..3 {
				render(&mut view, &mut state, vec![]);
			}
			let texts = render(&mut view, &mut state, vec![]);
			assert!(
				texts
					.iter()
					.any(|(text, _)| text.trim_end() == "Supported text remains")
			);
			for (label, present) in [
				("Poll · Preview unavailable", extra.poll),
				(
					"Sticker · Preview unavailable",
					extra.sticker_items || extra.stickers,
				),
				(
					"Components · Preview unavailable",
					extra.components || extra.components_v2,
				),
				("Open in Discord", extra.any()),
			] {
				assert_eq!(
					texts.iter().filter(|(text, _)| text == label).count(),
					usize::from(present),
					"{label}"
				);
			}
			assert!(
				!texts
					.iter()
					.any(|(text, _)| text == "System content · Preview unavailable")
			);
			assert!(view.opening.is_none());
			if extra == all {
				full_height = Some(view.heights[&message.id].1);
				let point = texts
					.iter()
					.find(|(text, _)| text == "Open in Discord")
					.unwrap()
					.1
					.center();
				for pressed in [true, false] {
					render(
						&mut view,
						&mut state,
						vec![
							egui::Event::PointerMoved(point),
							egui::Event::PointerButton {
								pos: point,
								button: egui::PointerButton::Primary,
								pressed,
								modifiers: egui::Modifiers::NONE,
							},
						],
					);
				}
				assert_eq!(
					view.browser_opening,
					discord_url(&channel, Some(message.id))
				);
				view.browser_opening = None;
			}
			if !extra.any() {
				assert!(
					view.heights[&message.id].1 < full_height.unwrap(),
					"Removing marker-only metadata must shrink the row"
				);
			}
		}
	}

	#[test]
	fn hover_actions_keep_layout_stable_and_support_keyboard_reply() {
		fn texts(shape: &egui::Shape, out: &mut Vec<(String, egui::Rect)>) {
			match shape {
				egui::Shape::Text(t) => out.push((
					t.galley.job.text.clone(),
					t.galley.rect.translate(t.pos.to_vec2()),
				)),
				egui::Shape::Vec(shapes) => {
					for shape in shapes {
						texts(shape, out);
					}
				}
				_ => {}
			}
		}
		for (width, dark) in [(900.0, true), (360.0, false)] {
			let ctx = egui::Context::default();
			crate::design::apply(&ctx);
			ctx.set_visuals(if dark {
				egui::Visuals::dark()
			} else {
				egui::Visuals::light()
			});
			let mut state = test_support::demo_state();
			state.timeline.clear();
			state.read_state.reset();
			let first = text_message(1);
			state.user = Some(first.author.clone());
			state.timeline.insert(first, false, false).unwrap();
			let mut second = text_message(60_000 << 22);
			second.content = "Grouped continuation".into();
			second.edited = true;
			state.timeline.insert(second, false, false).unwrap();
			let mut view = TimelineView::default();
			let mut avatars = crate::avatars::Avatars::default();
			let mut editing = None;
			let mut render =
				|view: &mut TimelineView, state: &mut State, events: Vec<egui::Event>| {
					let output = ctx.run_ui(
						egui::RawInput {
							screen_rect: Some(egui::Rect::from_min_size(
								egui::Pos2::ZERO,
								egui::vec2(width, 600.0),
							)),
							events,
							..Default::default()
						},
						|ui| {
							view.show(
								ui,
								state,
								&mut editing,
								&mut None,
								(
									&mut avatars,
									&mut crate::profiles::ProfileSession::default(),
								),
								None,
							)
						},
					);
					let mut painted = vec![];
					for shape in &output.shapes {
						texts(&shape.shape, &mut painted);
					}
					output.drop_without_applying_deltas();
					painted
				};
			for _ in 0..5 {
				render(&mut view, &mut state, vec![]);
			}
			let idle = render(&mut view, &mut state, vec![]);
			assert!(
				!idle
					.iter()
					.any(|(t, _)| t == "00:01" || t == "↩" || t == "✎")
			);
			assert!(idle.iter().any(|(t, _)| t == "(edited)"));
			let row = idle
				.iter()
				.find(|(t, _)| t.contains("Grouped continuation"))
				.unwrap()
				.1;
			let heights = view.heights.clone();
			let hovered = render(
				&mut view,
				&mut state,
				vec![egui::Event::PointerMoved(row.center())],
			);
			assert!(hovered.iter().any(|(t, _)| t == "00:01"));
			// Three quick reactions, add reaction, reply, forward, edit and the menu.
			assert_eq!(view.toolbar.unwrap().1.width(), 240.0);
			assert_eq!(view.heights, heights);
			let point = view.toolbar.unwrap().1.left_top() + egui::vec2(134.0, 14.0);
			for pressed in [true, false] {
				render(
					&mut view,
					&mut state,
					vec![
						egui::Event::PointerMoved(point),
						egui::Event::PointerButton {
							pos: point,
							button: egui::PointerButton::Primary,
							pressed,
							modifiers: egui::Modifiers::NONE,
						},
					],
				);
			}
			assert_eq!(
				state.reply.take(),
				Some(client_core::Reply::to(Id(60_000 << 22)))
			);
			// Both entry points share the same menu, including on selectable text,
			// row whitespace, and Shift+right-click (which must not quick-delete).
			let menu_labels = [
				"Copy message",
				"Reply",
				"Mark read through here",
				"Mark Unread",
				"Pin message",
				"Edit message",
				"Delete message\u{2026}",
			];
			for (button, point, modifiers) in [
				(
					egui::PointerButton::Primary,
					view.toolbar.unwrap().1.right_center() - egui::vec2(14.0, 0.0),
					egui::Modifiers::NONE,
				),
				(
					egui::PointerButton::Secondary,
					row.center(),
					egui::Modifiers::NONE,
				),
				(
					egui::PointerButton::Secondary,
					egui::pos2(width - 30.0, row.center().y),
					egui::Modifiers::SHIFT,
				),
			] {
				for pressed in [true, false] {
					render(
						&mut view,
						&mut state,
						vec![
							egui::Event::PointerMoved(point),
							egui::Event::PointerButton {
								pos: point,
								button,
								pressed,
								modifiers,
							},
							egui::Event::ModifiersChanged(modifiers),
						],
					);
				}
				render(&mut view, &mut state, vec![]);
				let labels = render(&mut view, &mut state, vec![]);
				assert_eq!(
					labels
						.iter()
						.filter_map(|(label, _)| menu_labels
							.contains(&label.as_str())
							.then_some(label.as_str()))
						.collect::<Vec<_>>(),
					menu_labels,
				);
				let reply = labels
					.iter()
					.find(|(label, _)| label == "Reply")
					.unwrap()
					.1
					.center();
				for pressed in [true, false] {
					render(
						&mut view,
						&mut state,
						vec![
							egui::Event::PointerMoved(reply),
							egui::Event::ModifiersChanged(egui::Modifiers::NONE),
							egui::Event::PointerButton {
								pos: reply,
								button: egui::PointerButton::Primary,
								pressed,
								modifiers: egui::Modifiers::NONE,
							},
						],
					);
				}
				assert_eq!(
					state.reply.take(),
					Some(client_core::Reply::to(Id(60_000 << 22)))
				);
				assert!(!egui::Popup::is_any_open(&ctx));
				assert!(view.quick_delete.is_none());
				assert_eq!(view.heights, heights);
			}
			ctx.memory_mut(|m| {
				if let Some(id) = m.focused() {
					m.surrender_focus(id);
				}
			});
			render(&mut view, &mut state, vec![egui::Event::PointerGone]);
			// Tab reaches an avatar, its message row, then reaction and reply actions.
			let key = |key| egui::Event::Key {
				key,
				physical_key: None,
				pressed: true,
				repeat: false,
				modifiers: egui::Modifiers::NONE,
			};
			for _ in 0..12 {
				render(&mut view, &mut state, vec![key(egui::Key::Tab)]);
				let focused = ctx
					.memory(|m| m.focused())
					.and_then(|id| ctx.read_response(id));
				// Reply follows the quick reactions and add reaction in the retained toolbar.
				if focused.is_some_and(|r| {
					view.toolbar.is_some_and(|(_, toolbar)| {
						toolbar.contains_rect(r.rect)
							&& r.rect.width() < 40.0
							&& (r.rect.left() - (toolbar.left() + 120.0)).abs() < 3.0
					})
				}) {
					render(&mut view, &mut state, vec![key(egui::Key::Enter)]);
					break;
				}
			}
			assert!(
				state.reply.is_some(),
				"Keyboard navigation must reach Reply"
			);
		}
	}
	#[test]
	fn quick_reactions_shift_actions_and_double_click_use_existing_paths() {
		fn texts(shape: &egui::Shape, out: &mut Vec<(String, egui::Rect)>) {
			match shape {
				egui::Shape::Text(t) => out.push((
					t.galley.job.text.clone(),
					t.galley.rect.translate(t.pos.to_vec2()),
				)),
				egui::Shape::Vec(shapes) => {
					for shape in shapes {
						texts(shape, out);
					}
				}
				_ => {}
			}
		}
		let ctx = egui::Context::default();
		crate::design::apply(&ctx);
		let mut state = test_support::demo_state();
		state.timeline.clear();
		state.read_state.reset();
		let id = Id(60_000 << 22);
		let mut message = text_message(id.0);
		message.content = "Short".into();
		state.user = Some(message.author.clone());
		state.timeline.insert(message, false, false).unwrap();
		let mut view = TimelineView {
			quick_reactions: Some(["🚀", "👍", "❤️"]),
			double_click_reaction: Some("🎉"),
			..Default::default()
		};
		let mut avatars = crate::avatars::Avatars::default();
		let clock = std::cell::Cell::new(0.0);
		let mut render = |view: &mut TimelineView,
		                  state: &mut State,
		                  mut events: Vec<egui::Event>,
		                  modifiers: egui::Modifiers| {
			events.insert(0, egui::Event::ModifiersChanged(modifiers));
			let output = ctx.run_ui(
				egui::RawInput {
					screen_rect: Some(egui::Rect::from_min_size(
						egui::Pos2::ZERO,
						egui::vec2(900.0, 600.0),
					)),
					time: Some(clock.replace(clock.get() + 1.0 / 60.0)),
					events,
					..Default::default()
				},
				|ui| {
					view.show(
						ui,
						state,
						&mut None,
						&mut None,
						(
							&mut avatars,
							&mut crate::profiles::ProfileSession::default(),
						),
						None,
					)
				},
			);
			let mut painted = vec![];
			for shape in &output.shapes {
				texts(&shape.shape, &mut painted);
			}
			let copied: Vec<String> = output
				.platform_output
				.commands
				.iter()
				.filter_map(|command| match command {
					egui::OutputCommand::CopyText(text) => Some(text.clone()),
					_ => None,
				})
				.collect();
			output.drop_without_applying_deltas();
			(painted, copied)
		};
		let click = |pos: egui::Pos2, modifiers: egui::Modifiers| {
			[true, false].map(|pressed| {
				vec![
					egui::Event::PointerMoved(pos),
					egui::Event::PointerButton {
						pos,
						button: egui::PointerButton::Primary,
						pressed,
						modifiers,
					},
				]
			})
		};
		let none = egui::Modifiers::NONE;
		for _ in 0..5 {
			render(&mut view, &mut state, vec![], none);
		}
		let (idle, _) = render(&mut view, &mut state, vec![], none);
		let text = idle.iter().find(|(t, _)| t == "Short").unwrap().1;
		render(
			&mut view,
			&mut state,
			vec![egui::Event::PointerMoved(text.center())],
			none,
		);
		let toolbar = view.toolbar.unwrap().1;
		assert_eq!(toolbar.width(), 240.0);
		// The first quick reaction toggles through the shared reaction path.
		for events in click(toolbar.left_center() + egui::vec2(14.0, 0.0), none) {
			render(&mut view, &mut state, events, none);
		}
		let rocket = model::ReactionEmoji {
			id: None,
			name: Some("🚀".into()),
		};
		assert_eq!(view.reaction.take(), Some((id, Some(rocket.clone()))));
		assert_eq!(view.quick_reaction_used.take(), Some("🚀"));

		// Shift hides quick reactions, adds direct actions and swaps the menu for quick delete.
		let shift = egui::Modifiers::SHIFT;
		render(
			&mut view,
			&mut state,
			vec![egui::Event::PointerMoved(text.center())],
			shift,
		);
		let toolbar = view.toolbar.unwrap().1;
		assert!(toolbar.width() >= 150.0 + 2.0 * 30.0, "{toolbar:?}");
		let mut copied = vec![];
		for events in click(toolbar.right_center() - egui::vec2(74.0, 0.0), shift) {
			copied.extend(render(&mut view, &mut state, events, shift).1);
		}
		assert_eq!(copied, [id.to_string()]);
		for events in click(toolbar.right_center() - egui::vec2(104.0, 0.0), shift) {
			copied.extend(render(&mut view, &mut state, events, shift).1);
		}
		assert_eq!(copied, [id.to_string(), "Short".into()]);
		for events in click(toolbar.right_center() - egui::vec2(44.0, 0.0), shift) {
			render(&mut view, &mut state, events, shift);
		}
		assert_eq!(view.quick_delete.take(), Some((Id(20), id)));
		render(&mut view, &mut state, vec![egui::Event::PointerGone], none);

		// Double-clicking text selects a word; beside it the independent preset toggles.
		for (pos, reacts) in [
			(text.center(), false),
			(egui::pos2(450.0, text.center().y), true),
		] {
			// Let the multi-click window lapse so this pair counts as a double click.
			clock.set(clock.get() + 2.0);
			for _ in 0..2 {
				for events in click(pos, none) {
					render(&mut view, &mut state, events, none);
				}
			}
			assert_eq!(
				view.reaction.take(),
				reacts.then(|| (
					id,
					Some(model::ReactionEmoji {
						id: None,
						name: Some("🎉".into())
					})
				)),
				"{pos:?}"
			);
			render(&mut view, &mut state, vec![egui::Event::PointerGone], none);
		}
	}
	#[test]
	fn right_clicking_a_reaction_opens_its_details_without_the_message_menu() {
		fn texts(shape: &egui::Shape, out: &mut Vec<(String, egui::Rect)>) {
			match shape {
				egui::Shape::Text(text) => out.push((
					text.galley.job.text.clone(),
					text.galley.rect.translate(text.pos.to_vec2()),
				)),
				egui::Shape::Vec(shapes) => {
					for shape in shapes {
						texts(shape, out);
					}
				}
				_ => {}
			}
		}
		let ctx = egui::Context::default();
		crate::design::apply(&ctx);
		let mut state = test_support::demo_state();
		state.timeline.clear();
		state.read_state.reset();
		let mut message = text_message(1);
		message.reactions = Some(vec![model::Reaction {
			emoji: model::ReactionEmoji {
				id: None,
				name: Some("\u{1f44d}".into()),
			},
			count: 3,
			me: false,
			me_burst: false,
		}]);
		state.timeline.insert(message, false, false).unwrap();
		let mut view = TimelineView::default();
		let mut avatars = crate::avatars::Avatars::default();
		let mut editing = None;
		let mut render = |view: &mut TimelineView, state: &mut State, events: Vec<egui::Event>| {
			let output = ctx.run_ui(
				egui::RawInput {
					screen_rect: Some(egui::Rect::from_min_size(
						egui::Pos2::ZERO,
						egui::vec2(900.0, 600.0),
					)),
					events,
					..Default::default()
				},
				|ui| {
					view.show(
						ui,
						state,
						&mut editing,
						&mut None,
						(
							&mut avatars,
							&mut crate::profiles::ProfileSession::default(),
						),
						None,
					)
				},
			);
			let mut painted = vec![];
			for shape in &output.shapes {
				texts(&shape.shape, &mut painted);
			}
			output.drop_without_applying_deltas();
			painted
		};
		for _ in 0..4 {
			render(&mut view, &mut state, vec![]);
		}
		let painted = render(&mut view, &mut state, vec![]);
		let reaction = painted
			.iter()
			.find(|(label, _)| label.starts_with('\u{1f44d}'))
			.unwrap_or_else(|| panic!("Missing reaction: {painted:?}"))
			.1
			.center();
		view.reaction_users = None;
		for pressed in [true, false] {
			render(
				&mut view,
				&mut state,
				vec![
					egui::Event::PointerMoved(reaction),
					egui::Event::PointerButton {
						pos: reaction,
						button: egui::PointerButton::Secondary,
						pressed,
						modifiers: egui::Modifiers::NONE,
					},
				],
			);
		}
		assert!(
			matches!(view.reaction_users, Some((Id(1), _, true))),
			"The right click must open the reaction details"
		);
		render(&mut view, &mut state, vec![]);
		let labels = render(&mut view, &mut state, vec![]);
		for menu in ["Copy message", "Reply", "Pin message"] {
			assert!(
				!labels.iter().any(|(label, _)| label == menu),
				"Reaction right click also opened the message menu: {labels:?}"
			);
		}
		assert!(!egui::Popup::is_any_open(&ctx));
	}
	#[test]
	fn reply_target_browsing_waits_for_success_and_explicit_latest_before_acknowledging() {
		fn collect(shape: &egui::Shape, labels: &mut Vec<(String, egui::Rect)>) {
			match shape {
				egui::Shape::Text(text) => labels.push((
					text.galley.job.text.clone(),
					text.galley.rect.translate(text.pos.to_vec2()),
				)),
				egui::Shape::Vec(shapes) => {
					for shape in shapes {
						collect(shape, labels);
					}
				}
				_ => {}
			}
		}
		for (found, width) in [(true, 900.0), (false, 360.0)] {
			let ctx = egui::Context::default();
			let mut state = State {
				auth: client_core::auth::AuthState::Authenticated,
				gateway_connected: true,
				freshness: model::Freshness::Loading,
				history_pending: true,
				selected: Some(Id(20)),
				search_target: Some(Id(19)),
				channels: vec![model::Channel {
					id: Id(20),
					guild: None,
					parent_id: None,
					position: 0,
					name: "Synthetic reply conversation".into(),
					kind: 1,
					recipients: vec![],
					member_list_id: None,
					tags: None,
					message_count: None,
					icon: None,
					last_message: Some(Id(20)),
				}],
				..Default::default()
			};
			state
				.timeline
				.insert(text_message(20), false, false)
				.unwrap();
			if found {
				state
					.timeline
					.insert(text_message(19), false, false)
					.unwrap();
			}
			let mut view = TimelineView::default();
			let mut avatars = crate::avatars::Avatars::default();
			let mut frame = |view: &mut TimelineView, state: &mut State, events| {
				let output = ctx.run_ui(
					egui::RawInput {
						focused: true,
						screen_rect: Some(egui::Rect::from_min_size(
							egui::Pos2::ZERO,
							egui::vec2(width, 600.0),
						)),
						events,
						..Default::default()
					},
					|ui| {
						view.show(
							ui,
							state,
							&mut None,
							&mut None,
							(
								&mut avatars,
								&mut crate::profiles::ProfileSession::default(),
							),
							None,
						)
					},
				);
				assert!(
					output
						.platform_output
						.commands
						.iter()
						.all(|command| matches!(
							command,
							egui::OutputCommand::TextSelectionSettled(_)
						)),
					"{:?}",
					output.platform_output.commands
				);
				let mut labels = vec![];
				for shape in &output.shapes {
					collect(&shape.shape, &mut labels);
				}
				output.drop_without_applying_deltas();
				labels
			};
			for _ in 0..3 {
				frame(&mut view, &mut state, vec![]);
			}
			assert_eq!(state.search_target, Some(Id(19)));
			assert!(view.mark_read.is_none());
			state.freshness = model::Freshness::Stale;
			assert!(view.highlighted.is_none());
			state.history_pending = false;
			state.status = "Synthetic history failure";
			frame(&mut view, &mut state, vec![]);
			assert_eq!(state.status, "Synthetic history failure");
			assert_eq!(state.search_target, Some(Id(19)));
			assert!(view.mark_read.is_none());
			state.freshness = model::Freshness::Fresh;
			state.revision += 1;
			for _ in 0..3 {
				frame(&mut view, &mut state, vec![]);
			}
			assert!(state.search_target.is_none());
			assert_eq!(view.highlighted.map(|(id, _)| id), found.then_some(Id(19)));
			if let Some((_, until)) = &mut view.highlighted {
				*until = -1.0;
			}
			frame(&mut view, &mut state, vec![]);
			assert!(view.highlighted.is_none());
			assert!(view.target_browsing && !view.following);
			assert!(view.mark_read.is_none());
			if !found {
				assert!(state.status.starts_with("Message was not returned"));
			}
			// The whole page fits onscreen, but only an explicit latest action resumes auto-read.
			frame(&mut view, &mut state, vec![]);
			let pos = view
				.present_control
				.expect("jump to present control")
				.center();
			for pressed in [true, false] {
				frame(
					&mut view,
					&mut state,
					vec![
						egui::Event::PointerMoved(pos),
						egui::Event::PointerButton {
							pos,
							button: egui::PointerButton::Primary,
							pressed,
							modifiers: egui::Modifiers::NONE,
						},
					],
				);
			}
			frame(&mut view, &mut state, vec![]);
			assert!(!view.target_browsing);
			assert_eq!(view.mark_read.take(), Some(Id(20)));
		}
	}

	fn unread_servers() -> State {
		use model::permissions as p;
		let channels = [(10, 1), (11, 1), (20, 2)]
			.into_iter()
			.map(|(id, guild)| model::Channel {
				id: Id(id),
				guild: Some(Id(guild)),
				parent_id: None,
				position: id as i32,
				name: format!("synthetic-{id}"),
				kind: 0,
				recipients: vec![],
				member_list_id: None,
				tags: None,
				message_count: None,
				icon: None,
				last_message: None,
			})
			.collect::<Vec<_>>();
		let permission_channels = channels
			.iter()
			.filter_map(|channel| {
				channel.guild.map(|guild| p::Channel {
					id: channel.id,
					guild,
					overwrites: Some(vec![]),
				})
			})
			.collect();
		let mut state = State {
			auth: client_core::auth::AuthState::Authenticated,
			gateway_connected: true,
			user: Some(model::User {
				id: Id(999),
				name: "Synthetic".into(),
				avatar: None,
				webhook: false,
				kind: Default::default(),
				discriminator: 0,
				primary_guild: None,
			}),
			guilds: [1, 2]
				.into_iter()
				.map(|id| model::Guild {
					default_message_notifications: None,
					stickers: None,
					emojis: None,
					id: Id(id),
					name: format!("Server {id}"),
					icon: None,
				})
				.collect(),
			channels,
			..State::default()
		};
		state
			.permissions
			.replace(p::Snapshot {
				guilds: (1..=2)
					.map(|id| p::Guild {
						id: Id(id),
						owner: Some(Id(999)),
						member: Some(p::Member {
							roles: vec![],
							timeout_until: None,
						}),
						roles: Some(vec![p::Role {
							id: Id(id),
							name: String::new(),
							color: 0,
							secondary_color: None,
							tertiary_color: None,
							position: 0,
							hoist: false,
							bits: p::VIEW_CHANNEL | p::READ_MESSAGE_HISTORY,
						}]),
					})
					.collect(),
				channels: permission_channels,
			})
			.unwrap();
		state
			.apply_read_state(client_core::read_state::Event::Snapshot {
				entries: Some(vec![
					(Id(10), Some(Id(1)), 0),
					(Id(11), Some(Id(1)), 0),
					(Id(20), Some(Id(1)), 0),
				]),
				version: Some(1),
				partial: false,
			})
			.unwrap();
		state
	}

	fn deliver_unread(state: &mut State, channel: Id, latest: u64) {
		assert_eq!(state.selected, Some(channel));
		assert!(state.history_pending);
		let messages = [latest - 1, latest]
			.into_iter()
			.map(|id| {
				let mut message = test_support::message(id, channel);
				message.content = "Synthetic tall unread row\n\n".repeat(40);
				message
			})
			.collect();
		state.apply(client_core::Envelope {
			generation: state.generation,
			event: client_core::Event::History {
				channel,
				request: state.request,
				older: false,
				messages,
			},
		});
		assert_eq!(state.freshness, model::Freshness::Fresh);
		assert_eq!(state.unread(channel), Some(true));
	}

	fn settle_banner(
		ctx: &egui::Context,
		view: &mut TimelineView,
		state: &mut State,
	) -> Vec<(String, egui::Rect)> {
		let mut labels = vec![];
		for _ in 0..4 {
			labels = banner_frame(ctx, view, state, vec![], false);
		}
		labels
	}

	fn expect_unread_held(view: &TimelineView, labels: &[(String, egui::Rect)], step: &str) {
		assert!(
			view.hold_read_ack && view.mark_read.is_none() && view.following,
			"{step}: hold={} following={} mark_read={:?}",
			view.hold_read_ack,
			view.following,
			view.mark_read
		);
		assert!(
			labels.iter().any(|(text, _)| text == "Mark as read"),
			"{step} removed the unread banner: {labels:?}"
		);
	}

	#[test]
	fn server_switch_keeps_the_unread_banner_until_a_downward_scroll() {
		let ctx = egui::Context::default();
		let mut state = unread_servers();
		let mut view = TimelineView::default();

		assert!(matches!(
			state.select(Id(10)),
			Some(client_core::Command::History { .. })
		));
		deliver_unread(&mut state, Id(10), 101);
		let labels = settle_banner(&ctx, &mut view, &mut state);
		expect_unread_held(&view, &labels, "channel open");

		assert!(matches!(
			state.select(Id(11)),
			Some(client_core::Command::History { .. })
		));
		deliver_unread(&mut state, Id(11), 201);
		let labels = settle_banner(&ctx, &mut view, &mut state);
		expect_unread_held(&view, &labels, "channel switch");

		assert!(matches!(
			state.select(Id(10)),
			Some(client_core::Command::History { .. })
		));
		deliver_unread(&mut state, Id(10), 101);
		let labels = settle_banner(&ctx, &mut view, &mut state);
		expect_unread_held(&view, &labels, "channel return");

		assert!(matches!(
			state.select_guild(Id(2)),
			Some(client_core::Command::History {
				channel: Id(20),
				..
			})
		));
		deliver_unread(&mut state, Id(20), 301);
		let labels = settle_banner(&ctx, &mut view, &mut state);
		expect_unread_held(&view, &labels, "server open");

		assert!(matches!(
			state.select_guild(Id(1)),
			Some(client_core::Command::History {
				channel: Id(10),
				..
			})
		));
		deliver_unread(&mut state, Id(10), 101);
		let labels = settle_banner(&ctx, &mut view, &mut state);
		expect_unread_held(&view, &labels, "server return");

		banner_frame(
			&ctx,
			&mut view,
			&mut state,
			vec![
				egui::Event::PointerMoved(egui::pos2(450.0, 300.0)),
				egui::Event::MouseWheel {
					unit: egui::MouseWheelUnit::Point,
					delta: egui::vec2(0.0, -80.0),
					modifiers: egui::Modifiers::NONE,
					phase: egui::TouchPhase::Move,
					source: egui::MouseWheelSource::Unknown,
				},
			],
			false,
		);
		assert_eq!(view.mark_read, Some(Id(101)));
		assert!(!view.hold_read_ack);
	}

	#[test]
	fn initial_unread_join_waits_for_a_downward_reach() {
		for (marker, width, dark) in [
			(Some(Id(10)), 320.0, false),
			(None, 900.0, true),
			(Some(Id(19)), 900.0, true),
		] {
			let mut state = State {
				auth: client_core::auth::AuthState::Authenticated,
				gateway_connected: true,
				freshness: model::Freshness::Fresh,
				selected: Some(Id(20)),
				channels: vec![model::Channel {
					id: Id(20),
					guild: None,
					parent_id: None,
					position: 0,
					name: "Synthetic unread conversation".into(),
					kind: 1,
					recipients: vec![],
					member_list_id: None,
					tags: None,
					message_count: None,
					icon: None,
					last_message: Some(Id(20)),
				}],
				..Default::default()
			};
			for id in [19, 20] {
				state
					.timeline
					.insert(text_message(id), false, false)
					.unwrap();
			}
			state
				.apply_read_state(client_core::read_state::Event::Snapshot {
					entries: Some(vec![(Id(20), marker, 0)]),
					version: Some(1),
					partial: false,
				})
				.unwrap();
			let ctx = egui::Context::default();
			ctx.set_visuals(if dark {
				egui::Visuals::dark()
			} else {
				egui::Visuals::light()
			});
			let mut view = TimelineView::default();
			let mut avatars = crate::avatars::Avatars::default();
			let mut frame = |view: &mut TimelineView, state: &mut State, events| {
				let output = ctx.run_ui(
					egui::RawInput {
						focused: true,
						events,
						screen_rect: Some(egui::Rect::from_min_size(
							egui::Pos2::ZERO,
							egui::vec2(width, 600.0),
						)),
						..Default::default()
					},
					|ui| {
						view.show(
							ui,
							state,
							&mut None,
							&mut None,
							(
								&mut avatars,
								&mut crate::profiles::ProfileSession::default(),
							),
							None,
						);
						assert!(ui.min_rect().right() <= ui.max_rect().right() + 1.0);
					},
				);
				assert!(
					output
						.platform_output
						.commands
						.iter()
						.all(|command| matches!(
							command,
							egui::OutputCommand::TextSelectionSettled(_)
						))
				);
				output.drop_without_applying_deltas();
			};
			for _ in 0..3 {
				frame(&mut view, &mut state, vec![]);
			}
			assert!(view.following && view.hold_read_ack);
			assert!(
				view.mark_read.is_none(),
				"Opening an unread channel must not acknowledge until the user scrolls toward the bottom"
			);
			frame(
				&mut view,
				&mut state,
				vec![
					egui::Event::PointerMoved(egui::pos2(width / 2.0, 300.0)),
					egui::Event::MouseWheel {
						unit: egui::MouseWheelUnit::Point,
						delta: egui::vec2(0.0, -80.0),
						modifiers: egui::Modifiers::NONE,
						phase: egui::TouchPhase::Move,
						source: egui::MouseWheelSource::Unknown,
					},
				],
			);
			assert!(!view.hold_read_ack && view.following);
			assert_eq!(
				view.mark_read.take(),
				Some(Id(20)),
				"A downward reach at the live edge acknowledges the unread join"
			);
		}
	}

	#[test]
	fn divider_and_dismissed_banner_stay_put_when_messages_arrive_at_the_live_edge() {
		let mut state = State {
			auth: client_core::auth::AuthState::Authenticated,
			gateway_connected: true,
			freshness: model::Freshness::Fresh,
			older_exhausted: true,
			selected: Some(Id(20)),
			channels: vec![model::Channel {
				id: Id(20),
				guild: None,
				parent_id: None,
				position: 0,
				name: "Synthetic unread conversation".into(),
				kind: 1,
				recipients: vec![],
				member_list_id: None,
				tags: None,
				message_count: None,
				icon: None,
				last_message: Some(Id(40)),
			}],
			..Default::default()
		};
		for id in 11..=40 {
			state
				.timeline
				.insert(text_message(id), false, false)
				.unwrap();
		}
		let marker = |state: &mut State, read: u64, version: u64| {
			state
				.apply_read_state(client_core::read_state::Event::Snapshot {
					entries: Some(vec![(Id(20), Some(Id(read)), 0)]),
					version: Some(version),
					partial: false,
				})
				.unwrap();
		};
		marker(&mut state, 32, 1);
		let ctx = egui::Context::default();
		let mut view = TimelineView::default();
		let mut labels = Vec::new();
		for _ in 0..3 {
			labels = banner_frame(&ctx, &mut view, &mut state, vec![], false);
		}
		assert_eq!(view.unread_boundary, Some(Id(33)));
		assert!(
			labels.iter().any(|(text, _)| text
				.replace(['\u{2068}', '\u{2069}'], "")
				.starts_with("8 new messages since")),
			"the banner counts the unread section: {labels:?}"
		);
		// Reading down to the live edge acknowledges the section and dismisses its banner.
		banner_frame(
			&ctx,
			&mut view,
			&mut state,
			vec![
				egui::Event::PointerMoved(egui::pos2(450.0, 300.0)),
				egui::Event::MouseWheel {
					unit: egui::MouseWheelUnit::Point,
					delta: egui::vec2(0.0, -80.0),
					modifiers: egui::Modifiers::NONE,
					phase: egui::TouchPhase::Move,
					source: egui::MouseWheelSource::Unknown,
				},
			],
			false,
		);
		assert_eq!(view.mark_read.take(), Some(Id(40)));
		marker(&mut state, 40, 2);
		// Someone else posts while the reader watches the latest message.
		state
			.timeline
			.insert(text_message(41), false, false)
			.unwrap();
		state.channels[0].last_message = Some(Id(41));
		for _ in 0..3 {
			labels = banner_frame(&ctx, &mut view, &mut state, vec![], false);
		}
		assert_eq!(
			view.unread_boundary,
			Some(Id(33)),
			"the divider stays at the first message that was unread on arrival"
		);
		assert!(
			!labels.iter().any(|(text, _)| text.contains("new message")),
			"a seen arrival must not revive the dismissed banner: {labels:?}"
		);
		assert_eq!(view.mark_read.take(), Some(Id(41)));
	}

	/// A synthetic DM with messages 11..=40 loaded and a read marker at `read`.
	fn live_unread_channel(read: u64) -> State {
		let mut state = State {
			auth: client_core::auth::AuthState::Authenticated,
			gateway_connected: true,
			freshness: model::Freshness::Fresh,
			older_exhausted: true,
			selected: Some(Id(20)),
			channels: vec![model::Channel {
				id: Id(20),
				guild: None,
				parent_id: None,
				position: 0,
				name: "Synthetic live conversation".into(),
				kind: 1,
				recipients: vec![],
				member_list_id: None,
				tags: None,
				message_count: None,
				icon: None,
				last_message: Some(Id(40)),
			}],
			..Default::default()
		};
		for id in 11..=40 {
			state
				.timeline
				.insert(text_message(id), false, false)
				.unwrap();
		}
		set_read_marker(&mut state, read, 1);
		state
	}

	fn set_read_marker(state: &mut State, read: u64, version: u64) {
		state
			.apply_read_state(client_core::read_state::Event::Snapshot {
				entries: Some(vec![(Id(20), Some(Id(read)), 0)]),
				version: Some(version),
				partial: false,
			})
			.unwrap();
	}

	fn arrive(state: &mut State, id: u64) {
		if state.timeline.get(Id(id)).is_none() {
			state
				.timeline
				.insert(text_message(id), false, false)
				.unwrap();
		}
		state.channels[0].last_message = Some(Id(id));
		state.revision += 1;
	}

	fn plain(labels: &[(String, egui::Rect)]) -> Vec<String> {
		labels
			.iter()
			.map(|(text, _)| text.replace(['\u{2068}', '\u{2069}'], ""))
			.collect()
	}

	fn click_at(ctx: &egui::Context, view: &mut TimelineView, state: &mut State, pos: egui::Pos2) {
		for pressed in [true, false] {
			banner_frame(
				ctx,
				view,
				state,
				vec![
					egui::Event::PointerMoved(pos),
					egui::Event::PointerButton {
						pos,
						button: egui::PointerButton::Primary,
						pressed,
						modifiers: egui::Modifiers::NONE,
					},
				],
				false,
			);
		}
	}

	#[test]
	fn arrivals_at_a_focused_live_edge_are_read_without_a_banner() {
		let ctx = egui::Context::default();
		let mut state = live_unread_channel(40);
		let mut view = TimelineView::default();
		settle_banner(&ctx, &mut view, &mut state);
		assert!(view.following && !view.hold_read_ack);
		for (version, id) in [(2, 41), (3, 42), (4, 43)] {
			if id == 42 {
				// A tall arrival is measured after it lands; the reader still follows it.
				let mut tall = text_message(id);
				tall.content = "Synthetic tall live row\n\n".repeat(30);
				state.timeline.insert(tall, false, false).unwrap();
			}
			arrive(&mut state, id);
			for _ in 0..4 {
				let labels = plain(&banner_frame(&ctx, &mut view, &mut state, vec![], false));
				assert!(
					!labels
						.iter()
						.any(|text| text == "Mark as read" || text.contains("new message")),
					"a watched arrival raised the unread banner: {labels:?}"
				);
				assert!(
					view.following && view.present_control.is_none(),
					"{labels:?}"
				);
			}
			assert_eq!(
				view.mark_read.take(),
				Some(Id(id)),
				"the arrival is acknowledged"
			);
			set_read_marker(&mut state, id, version);
		}
		assert_eq!(
			view.unread_boundary, None,
			"watched arrivals add no divider"
		);
	}

	#[test]
	fn marking_read_resets_the_banner_and_later_arrivals_stay_read_at_the_bottom() {
		let ctx = egui::Context::default();
		let mut state = live_unread_channel(32);
		let mut view = TimelineView::default();
		let labels = settle_banner(&ctx, &mut view, &mut state);
		assert!(
			plain(&labels)
				.iter()
				.any(|text| text.starts_with("8 new messages since")),
			"{labels:?}"
		);
		let pos = labels
			.iter()
			.find(|(text, _)| text == "Mark as read")
			.map(|(_, rect)| rect.center())
			.expect("the unread banner offers Mark as read");
		click_at(&ctx, &mut view, &mut state, pos);
		assert_eq!(view.mark_channel_read.take(), Some(Id(20)));
		// A message arrives before the acknowledgement round trip completes, and another after.
		arrive(&mut state, 41);
		let labels = plain(&settle_banner(&ctx, &mut view, &mut state));
		assert!(
			!labels.iter().any(|text| text == "Mark as read"),
			"an arrival revived the read banner: {labels:?}"
		);
		set_read_marker(&mut state, 41, 2);
		view.mark_read = None;
		for (version, id) in [(3, 42), (4, 43)] {
			arrive(&mut state, id);
			let labels = plain(&settle_banner(&ctx, &mut view, &mut state));
			assert!(
				!labels
					.iter()
					.any(|text| text == "Mark as read" || text.contains("new message")),
				"an arrival at the bottom revived the banner: {labels:?}"
			);
			assert_eq!(view.mark_read.take(), Some(Id(id)));
			set_read_marker(&mut state, id, version);
		}
	}

	#[test]
	fn arrivals_while_scrolled_up_count_only_unseen_messages_and_offer_the_way_down() {
		let ctx = egui::Context::default();
		let mut state = live_unread_channel(32);
		let mut view = TimelineView::default();
		settle_banner(&ctx, &mut view, &mut state);
		// Read down to the live edge: the section is acknowledged and its banner leaves.
		banner_frame(
			&ctx,
			&mut view,
			&mut state,
			vec![
				egui::Event::PointerMoved(egui::pos2(450.0, 300.0)),
				egui::Event::MouseWheel {
					unit: egui::MouseWheelUnit::Point,
					delta: egui::vec2(0.0, -80.0),
					modifiers: egui::Modifiers::NONE,
					phase: egui::TouchPhase::Move,
					source: egui::MouseWheelSource::Unknown,
				},
			],
			false,
		);
		assert_eq!(view.mark_read.take(), Some(Id(40)));
		set_read_marker(&mut state, 40, 2);
		settle_banner(&ctx, &mut view, &mut state);
		// Scroll a screen up, then two messages arrive out of sight.
		let scroll_up = |view: &mut TimelineView, state: &mut State| {
			for _ in 0..4 {
				let total = view.rows.iter().map(|(_, height)| height).sum::<f32>();
				let offset = (total - 600.0 - 400.0).max(0.0);
				let (index, _, top) = visible_range(&view.rows, offset, offset);
				view.following = false;
				view.jump = false;
				view.anchor = Some((view.rows[index].0, offset - top));
				view.revision = u64::MAX;
				banner_frame(&ctx, view, state, vec![], false);
			}
		};
		scroll_up(&mut view, &mut state);
		assert!(!view.following);
		assert!(view.present_control.is_none(), "nothing new below yet");
		arrive(&mut state, 41);
		arrive(&mut state, 42);
		scroll_up(&mut view, &mut state);
		let labels = plain(&banner_frame(&ctx, &mut view, &mut state, vec![], false));
		assert!(
			labels
				.iter()
				.any(|text| text.starts_with("2 new messages since")),
			"the banner counts only the unseen arrivals: {labels:?}"
		);
		assert_eq!(
			view.unread_boundary,
			Some(Id(41)),
			"unseen arrivals start a new section"
		);
		assert!(
			labels.iter().any(|text| text == "2 new messages"),
			"the way down names the unseen arrivals: {labels:?}"
		);
		assert!(view.mark_read.is_none(), "unseen arrivals stay unread");
		let pos = view.present_control.expect("a way back down").center();
		click_at(&ctx, &mut view, &mut state, pos);
		for _ in 0..30 {
			banner_frame(&ctx, &mut view, &mut state, vec![], false);
		}
		assert!(view.following && view.present_control.is_none());
		assert_eq!(
			view.mark_read.take(),
			Some(Id(42)),
			"reaching the bottom reads them"
		);
		set_read_marker(&mut state, 42, 3);
		let labels = plain(&settle_banner(&ctx, &mut view, &mut state));
		assert!(
			!labels
				.iter()
				.any(|text| text == "Mark as read" || text.contains("new message")),
			"reaching the bottom reset the banner: {labels:?}"
		);
	}

	#[test]
	fn auto_read_requires_focused_latest_and_does_not_retry_failed_marker() {
		let mut state = State {
			auth: client_core::auth::AuthState::Authenticated,
			gateway_connected: true,
			freshness: model::Freshness::Fresh,
			selected: Some(Id(20)),
			channels: vec![model::Channel {
				id: Id(20),
				guild: None,
				parent_id: None,
				position: 0,
				name: "Synthetic DM".into(),
				kind: 1,
				recipients: vec![],
				member_list_id: None,
				tags: None,
				message_count: None,
				icon: None,
				last_message: Some(Id(1)),
			}],
			..Default::default()
		};
		state
			.timeline
			.insert(text_message(1), false, false)
			.unwrap();
		let ctx = egui::Context::default();
		let mut view = TimelineView::default();
		let mut avatars = crate::avatars::Avatars::default();
		let mut frame = |view: &mut TimelineView, state: &mut State, focused| {
			ctx.run_ui(
				egui::RawInput {
					focused,
					..Default::default()
				},
				|ui| {
					view.show(
						ui,
						state,
						&mut None,
						&mut None,
						(
							&mut avatars,
							&mut crate::profiles::ProfileSession::default(),
						),
						None,
					);
				},
			)
			.drop_without_applying_deltas();
		};
		frame(&mut view, &mut state, false);
		assert!(view.mark_read.is_none());
		frame(&mut view, &mut state, true);
		assert_eq!(view.mark_read.take(), Some(Id(1)));
		frame(&mut view, &mut state, true);
		assert!(view.mark_read.is_none());
		state.channels[0].last_message = Some(Id(3));
		state
			.timeline
			.insert(text_message(2), false, false)
			.unwrap();
		state.revision += 1;
		state.history_before = Some(Id(2));
		frame(&mut view, &mut state, true);
		assert!(
			view.mark_read.is_none(),
			"Historical window is not the latest message"
		);
		// The latest page is the live edge; latest metadata beyond it outlived a deletion.
		state.history_before = None;
		state.revision += 1;
		frame(&mut view, &mut state, true);
		assert_eq!(view.mark_read.take(), Some(Id(3)));
	}
	#[test]
	fn underestimated_leading_row_does_not_hide_history_or_inflate_scroll_extent() {
		fn contains_final_row(shape: &egui::Shape) -> bool {
			match shape {
				egui::Shape::Text(text) => text.galley.job.text.contains("Visible final row"),
				egui::Shape::Vec(shapes) => shapes.iter().any(contains_final_row),
				_ => false,
			}
		}

		let mut state = State {
			selected: Some(Id(20)),
			revision: 1,
			demo: true,
			..Default::default()
		};
		for id in 1..=4 {
			let mut message = text_message(id);
			message.content = if id == 1 {
				"Tall leading row\n".repeat(80)
			} else if id == 4 {
				"Visible final row".into()
			} else {
				"Visible anchor row".into()
			};
			state.timeline.insert(message, false, false).unwrap();
		}
		let ctx = egui::Context::default();
		crate::design::apply(&ctx);
		let mut view = TimelineView::default();
		let mut avatars = crate::avatars::Avatars::default();
		let mut frame_number = 0;
		let mut frame = |view: &mut TimelineView, state: &mut State| {
			frame_number += 1;
			ctx.run_ui(
				egui::RawInput {
					time: Some(f64::from(frame_number) / 60.0),
					screen_rect: Some(egui::Rect::from_min_size(
						egui::Pos2::ZERO,
						egui::vec2(900.0, 600.0),
					)),
					..Default::default()
				},
				|ui| {
					view.show(
						ui,
						state,
						&mut None,
						&mut None,
						(
							&mut avatars,
							&mut crate::profiles::ProfileSession::default(),
						),
						None,
					)
				},
			)
		};
		for _ in 0..8 {
			frame(&mut view, &mut state).drop_without_applying_deltas();
		}

		// Force a severe underestimate without invalidating the row key, as can
		// happen when content geometry changes independently of its message data.
		let key = row_key(
			state.timeline.get(Id(1)).unwrap(),
			None,
			view.unread_boundary,
			&state,
		);
		view.heights.insert(Id(1), (key, 76.0));
		view.following = false;
		// The extra short row before the anchor also catches premature cutoff while
		// the leading measurement cursor is still below the visible viewport.
		view.anchor = Some((Id(3), 5.0));
		view.revision = u64::MAX;
		let output = frame(&mut view, &mut state);
		let final_visible = output
			.shapes
			.iter()
			.any(|shape| shape.clip_rect.is_positive() && contains_final_row(&shape.shape));
		output.drop_without_applying_deltas();
		assert!(
			view.heights[&Id(1)].1 > 600.0,
			"Fixture must measure a tall leading row"
		);
		assert!(
			final_visible,
			"Leading measurement must not blank the visible rows"
		);
		assert!(
			view.following,
			"The compensated short content must reach its real bottom; hidden leading bounds must not create phantom scroll space"
		);
	}

	#[test]
	fn wheel_scrolling_keeps_visible_messages_stable_during_measurement() {
		fn texts(shape: &egui::Shape, out: &mut BTreeMap<String, f32>) {
			match shape {
				egui::Shape::Text(text) if text.galley.job.text.starts_with("Row ") => {
					out.insert(text.galley.job.text.clone(), text.pos.y);
				}
				egui::Shape::Vec(shapes) => {
					for shape in shapes {
						texts(shape, out);
					}
				}
				_ => {}
			}
		}
		let mut worst_error = 0.0_f32;
		for width in [900.0, 360.0] {
			let mut state = State {
				selected: Some(Id(20)),
				revision: 1,
				demo: true,
				..Default::default()
			};
			for id in 1..=500 {
				let mut message = text_message(id);
				message.content = format!("Row {id}: {}", message.content);
				if id % 7 == 0 {
					message
						.content
						.push_str(&" Long wrapping content.".repeat(24));
				}
				state.timeline.insert(message, false, false).unwrap();
			}
			let ctx = egui::Context::default();
			crate::design::apply(&ctx);
			let mut view = TimelineView {
				instant_scrolling: true,
				..Default::default()
			};
			let mut avatars = crate::avatars::Avatars::default();
			let mut frame_number = 0;
			let mut frame = |view: &mut TimelineView, delta: f32| {
				frame_number += 1;
				let output = ctx.run_ui(
					egui::RawInput {
						time: Some(f64::from(frame_number) / 60.0),
						screen_rect: Some(egui::Rect::from_min_size(
							egui::Pos2::ZERO,
							egui::vec2(width, 600.0),
						)),
						events: vec![
							egui::Event::PointerMoved(egui::pos2(150.0, 200.0)),
							egui::Event::MouseWheel {
								unit: egui::MouseWheelUnit::Point,
								delta: egui::vec2(0.0, delta),
								modifiers: egui::Modifiers::NONE,
								phase: egui::TouchPhase::Move,
								source: egui::MouseWheelSource::Unknown,
							},
						],
						..Default::default()
					},
					|ui| {
						crate::scroll::apply_preferences(
							ui.ctx(),
							model::ReadingPreferences {
								smooth_scrolling: false,
								..Default::default()
							},
						);
						view.show(
							ui,
							&mut state,
							&mut None,
							&mut None,
							(
								&mut avatars,
								&mut crate::profiles::ProfileSession::default(),
							),
							None,
						)
					},
				);
				let mut labels = BTreeMap::new();
				for shape in &output.shapes {
					if shape.clip_rect.is_positive() {
						texts(&shape.shape, &mut labels);
					}
				}
				output.drop_without_applying_deltas();
				labels
			};
			for _ in 0..8 {
				frame(&mut view, 0.0);
			}
			assert!(view.instant_scrolling);
			view.following = false;
			view.anchor = Some((Id(200), 5.0));
			view.revision = u64::MAX;
			for _ in 0..8 {
				frame(&mut view, 0.0);
			}
			let mut max_error = 0.0_f32;
			// Small point deltas bypass wheel smoothing; each presented frame must move
			// the same message by four points, including frames that discover new rows.
			for (delta, frames) in [(4.0, 120), (-4.0, 240), (0.0, 4)] {
				let mut previous = frame(&mut view, delta);
				let mut comparisons = 0;
				for _ in 0..frames {
					let current = frame(&mut view, delta);
					for (text, y) in &current {
						if (50.0..500.0).contains(y)
							&& let Some(before) = previous.get(text)
						{
							max_error = max_error.max((y - before - delta).abs());
							comparisons += 1;
						}
					}
					previous = current;
				}
				assert!(
					comparisons >= frames,
					"Every frame needs visible message evidence"
				);
			}
			println!("width={width}, maximum scroll displacement error={max_error:.3}pt");
			worst_error = worst_error.max(max_error);
		}
		assert!(
			worst_error < 1.0,
			"wheel movement must not bounce during reflow"
		);
	}
	#[test]
	fn native_layout_virtualizes_preserves_anchor_and_jumps_after_scrolling() {
		for (width, dark) in [(900.0, true), (360.0, false)] {
			let mut state = State {
				selected: Some(Id(20)),
				revision: 1,
				demo: true,
				..Default::default()
			};
			for id in 1..=500 {
				state
					.timeline
					.insert(text_message(id), false, false)
					.unwrap();
			}
			let ctx = egui::Context::default();
			crate::design::apply(&ctx);
			ctx.set_visuals(if dark {
				egui::Visuals::dark()
			} else {
				egui::Visuals::light()
			});
			let mut view = TimelineView::default();
			let mut avatars = crate::avatars::Avatars::default();
			let mut render = |view: &mut TimelineView, state: &mut State| {
				ctx.run_ui(
					egui::RawInput {
						screen_rect: Some(egui::Rect::from_min_size(
							egui::Pos2::ZERO,
							egui::vec2(width, 600.0),
						)),
						..Default::default()
					},
					|ui| {
						view.show(
							ui,
							state,
							&mut None,
							&mut None,
							(
								&mut avatars,
								&mut crate::profiles::ProfileSession::default(),
							),
							None,
						);
					},
				)
				.drop_without_applying_deltas();
			};
			for _ in 0..8 {
				render(&mut view, &mut state);
			}
			assert!(view.following);
			assert!(
				view.heights.len() < 60,
				"Only visible rows and overscan are measured, got {}",
				view.heights.len()
			);
			state
				.timeline
				.insert(text_message(501), false, false)
				.unwrap();
			state.revision += 1;
			render(&mut view, &mut state);
			assert!(
				view.following,
				"An arriving message must keep the live edge sticky"
			);
			view.following = false;
			view.anchor = Some((Id(200), 5.0));
			view.revision = u64::MAX;
			for _ in 0..8 {
				render(&mut view, &mut state);
			}
			assert!(!view.following);
			let anchor = view.anchor.unwrap();
			assert_eq!(anchor.0, Id(200));
			// A newly measured leading row while browsing must not opt into the
			// bottom restoration used for a following layout retry.
			let (key, height) = view.heights[&Id(199)];
			assert!(height > 1.0);
			let reflows = view.reflow_frames;
			view.heights.insert(Id(199), (key, 1.0));
			view.revision = u64::MAX;
			render(&mut view, &mut state);
			assert!(view.reflow_frames > reflows);
			assert!(!view.following && !view.jump);
			assert_eq!(view.anchor, Some(anchor));
			crate::MessagingUi::default().apply_reading_preferences(
				&ctx,
				model::ReadingPreferences {
					zoom_percent: 125,
					..Default::default()
				},
			);
			for _ in 0..8 {
				render(&mut view, &mut state);
			}
			assert_eq!(
				view.anchor.unwrap().0,
				anchor.0,
				"Reading zoom preserves the anchored message"
			);
			state.timeline.insert(text_message(0), false, true).unwrap();
			state.revision += 1;
			for _ in 0..4 {
				render(&mut view, &mut state);
			}
			assert_eq!(view.anchor.unwrap().0, anchor.0);
			state.set_preserve_deleted_messages(true);
			state.timeline.delete(anchor.0).unwrap();
			state.revision += 1;
			for _ in 0..4 {
				render(&mut view, &mut state);
			}
			assert_eq!(
				view.anchor.unwrap().0,
				anchor.0,
				"Deleting the anchored row keeps that message visible"
			);
			assert!(view.heights.contains_key(&anchor.0));
			view.jump = true;
			view.following = true;
			for _ in 0..8 {
				render(&mut view, &mut state);
			}
			assert!(
				view.following,
				"Explicit jump must override persisted scroll state"
			);
		}
	}
	#[test]
	fn switching_cached_chats_keeps_message_positions_stable() {
		for (width, count) in [(900.0, 2), (360.0, 50)] {
			let mut state = test_support::demo_state();
			let ctx = egui::Context::default();
			crate::design::apply(&ctx);
			let mut view = TimelineView::default();
			let mut avatars = crate::avatars::Avatars::default();
			let mut render = |state: &mut State, events: Vec<egui::Event>| {
				let output = ctx.run_ui(
					egui::RawInput {
						events,
						screen_rect: Some(egui::Rect::from_min_size(
							egui::Pos2::ZERO,
							egui::vec2(width, 480.0),
						)),
						..Default::default()
					},
					|ui| {
						view.show(
							ui,
							state,
							&mut None,
							&mut None,
							(
								&mut avatars,
								&mut crate::profiles::ProfileSession::default(),
							),
							None,
						)
					},
				);
				let y = output.shapes.iter().find_map(|shape| match &shape.shape {
					egui::Shape::Text(text) if text.galley.text().contains("Switch anchor") => {
						Some(text.pos.y)
					}
					_ => None,
				});
				output.drop_without_applying_deltas();
				(
					y,
					view.scroll_offset,
					view.following,
					view.hold_read_ack,
					view.anchor,
				)
			};
			for channel in [Id(20), Id(21)] {
				state.select(channel);
				state.history(None);
				let messages = (0..count)
					.map(|index| {
						let mut message = text_message(1000 * channel.0 + index);
						message.channel = channel;
						message.content = if index == count - 1 {
							"Switch anchor".into()
						} else {
							"A wrapped synthetic message with different measured and estimated heights. ".repeat(3)
						};
						message
					})
					.collect();
				state.apply(client_core::Envelope {
					generation: state.generation,
					event: client_core::Event::History {
						channel,
						request: state.request,
						older: false,
						messages,
					},
				});
				for _ in 0..6 {
					render(&mut state, vec![]);
				}
			}
			for channel in [Id(20), Id(21), Id(20)] {
				assert!(matches!(
					state.select(channel),
					Some(client_core::Command::History { .. })
				));
				assert!(state.history_pending);
				let first = render(&mut state, vec![])
					.0
					.expect("cached switch must paint last message");
				for _ in 0..4 {
					let next = render(&mut state, vec![])
						.0
						.expect("settled chat must paint last message");
					assert!(
						(first - next).abs() <= 1.0,
						"cached switch moved from {first} to {next} at width {width}"
					);
				}
				let messages = state.timeline.iter().cloned().collect();
				state.apply(client_core::Envelope {
					generation: state.generation,
					event: client_core::Event::History {
						channel,
						request: state.request,
						older: false,
						messages,
					},
				});
				let refreshed = render(&mut state, vec![])
					.0
					.expect("refreshed chat must paint last message");
				assert!(
					(first - refreshed).abs() <= 1.0,
					"refresh moved from {first} to {refreshed} at width {width}"
				);
			}
			if count == 50 {
				let (_, bottom, _, _, _) = render(&mut state, vec![]);
				let up = vec![
					egui::Event::PointerMoved(egui::pos2(width / 2.0, 240.0)),
					egui::Event::MouseWheel {
						unit: egui::MouseWheelUnit::Point,
						delta: egui::vec2(0.0, 4_000.0),
						modifiers: egui::Modifiers::NONE,
						phase: egui::TouchPhase::Move,
						source: egui::MouseWheelSource::Unknown,
					},
				];
				let mut left = bottom;
				let mut pinned = None;
				for _ in 0..6 {
					let frame = render(&mut state, up.clone());
					left = frame.1;
					pinned = frame.4;
				}
				let (pinned_id, _) = pinned.expect("scrolled channel has an anchor");
				assert!(
					left + 200.0 < bottom,
					"scroll left the live edge at {left}, bottom was {bottom}"
				);
				state.select(Id(21));
				render(&mut state, vec![]);
				assert!(
					state.select(Id(20)).is_none(),
					"a parked page scrolls locally"
				);
				let mut back = (None, 0.0, true, false, None);
				for _ in 0..8 {
					back = render(&mut state, vec![]);
				}
				assert_eq!(back.4.map(|(id, _)| id), Some(pinned_id));
				assert!(
					(back.1 - left).abs() <= 1.0,
					"loaded return moved from {left} to {}",
					back.1
				);
				assert!(!back.2 && !back.3);
				state.select(Id(21));
				render(&mut state, vec![]);
				state.clear_cached_history();
				let command = state.select(Id(20));
				let client_core::Command::History {
					before: None,
					after: Some(after),
					..
				} = command.expect("an evicted page requests history")
				else {
					panic!("evicted return did not request the page after the pinned message");
				};
				let saved = state
					.reading(Id(20))
					.expect("evicted channel keeps its cursor");
				let pinned_id = saved.message.expect("evicted cursor names a message");
				assert_eq!(after.0, pinned_id.0 - 1);
				let pinned_inset = saved.inset;
				let messages = (0..40)
					.map(|index| {
						let mut message = text_message(pinned_id.0 + index);
						message.channel = Id(20);
						message.content = "A wrapped synthetic message with different measured and estimated heights. ".repeat(3);
						message
					})
					.collect();
				state.apply(client_core::Envelope {
					generation: state.generation,
					event: client_core::Event::History {
						channel: Id(20),
						request: state.request,
						older: false,
						messages,
					},
				});
				let mut back = (None, 0.0, true, false, None);
				for _ in 0..8 {
					back = render(&mut state, vec![]);
				}
				assert_eq!(back.4.map(|(id, _)| id), Some(pinned_id));
				assert!((back.1 - pinned_inset).abs() <= 1.0);
				assert!(!back.2 && !back.3);
			}
		}
	}

	#[test]
	fn resident_preview_renders_only_selected_rows_while_revalidating() {
		fn collect(shape: &egui::Shape, labels: &mut Vec<String>) {
			match shape {
				egui::Shape::Text(text) => labels.push(text.galley.job.text.clone()),
				egui::Shape::Vec(shapes) => {
					for shape in shapes {
						collect(shape, labels);
					}
				}
				_ => {}
			}
		}
		for (width, dark) in [(900.0, true), (360.0, false)] {
			for deleted_only in [false, true] {
				let mut state = test_support::demo_state();
				state.timeline.clear();
				state.history(None);
				let messages = (500..550)
					.map(|id| {
						let mut message = text_message(id);
						message.content = format!("Resident alpha row {id}");
						message
					})
					.collect();
				state.apply(client_core::Envelope {
					generation: state.generation,
					event: client_core::Event::History {
						channel: Id(20),
						request: state.request,
						older: false,
						messages,
					},
				});
				if deleted_only {
					state.set_preserve_deleted_messages(true);
					state.apply(client_core::Envelope {
						generation: state.generation,
						event: client_core::Event::DeleteBulk {
							channel: Id(20),
							ids: (500..550).map(Id).collect(),
						},
					});
				}
				let expected: Vec<_> = state.timeline.row_ids().collect();
				assert_eq!(expected.len(), 50);
				let ctx = egui::Context::default();
				crate::design::apply(&ctx);
				ctx.set_visuals(if dark {
					egui::Visuals::dark()
				} else {
					egui::Visuals::light()
				});
				let mut view = TimelineView::default();
				let mut avatars = crate::avatars::Avatars::default();
				let mut render = |view: &mut TimelineView, state: &mut State| {
					let output = ctx.run_ui(
						egui::RawInput {
							screen_rect: Some(egui::Rect::from_min_size(
								egui::Pos2::ZERO,
								egui::vec2(width, 480.0),
							)),
							..Default::default()
						},
						|ui| {
							view.show(
								ui,
								state,
								&mut None,
								&mut None,
								(
									&mut avatars,
									&mut crate::profiles::ProfileSession::default(),
								),
								None,
							);
							assert!(ui.min_rect().right() <= ui.max_rect().right() + 1.0);
						},
					);
					assert!(
						output
							.platform_output
							.commands
							.iter()
							.all(|command| matches!(
								command,
								egui::OutputCommand::TextSelectionSettled(_)
							))
					);
					let mut labels = vec![];
					for shape in &output.shapes {
						collect(&shape.shape, &mut labels);
					}
					output.drop_without_applying_deltas();
					labels
				};
				for _ in 0..6 {
					render(&mut view, &mut state);
				}
				assert!(matches!(
					state.select(Id(21)),
					Some(client_core::Command::History {
						channel: Id(21),
						before: None,
						..
					})
				));
				let labels = render(&mut view, &mut state);
				assert!(view.rows.is_empty());
				assert!(!labels.iter().any(|text| text.contains("Resident alpha")));
				let mut beta = text_message(900);
				beta.channel = Id(21);
				beta.content = "Resident beta content".into();
				state.apply(client_core::Envelope {
					generation: state.generation,
					event: client_core::Event::History {
						channel: Id(21),
						request: state.request,
						older: false,
						messages: vec![beta],
					},
				});
				for _ in 0..3 {
					render(&mut view, &mut state);
				}
				assert_eq!(
					view.rows.iter().map(|(id, _)| *id).collect::<Vec<_>>(),
					vec![Id(900)]
				);
				assert!(matches!(
					state.select(Id(20)),
					Some(client_core::Command::History {
						channel: Id(20),
						before: None,
						..
					})
				));
				assert_eq!(state.freshness, model::Freshness::Loading);
				assert!(state.history_pending);
				assert_eq!(state.timeline.row_ids().collect::<Vec<_>>(), expected);
				for _ in 0..6 {
					render(&mut view, &mut state);
				}
				let labels = render(&mut view, &mut state);
				assert!(!labels.iter().any(|text| text.contains("Resident beta")));
				assert_eq!(
					view.rows.iter().map(|(id, _)| *id).collect::<Vec<_>>(),
					expected
				);
				assert!(
					view.rows
						.iter()
						.all(|(_, height)| height.is_finite() && *height > 0.0)
				);
				assert!(
					view.following,
					"Resident navigation starts at the latest loaded row"
				);
				assert!(
					view.mark_read.is_none(),
					"Unrevalidated resident rows must not acknowledge read state"
				);
				assert!(labels.iter().any(|text| text.contains("Resident alpha")));
				if deleted_only {
					assert!(state.timeline.is_empty());
					assert!(!labels.iter().any(|text| text == "Message deleted"));
				}
			}
		}
	}

	#[test]
	fn mixed_height_virtualization_visits_only_viewport() {
		let rows: Vec<_> = (1..=500)
			.map(|id| (Id(id), if id % 2 == 0 { 100.0 } else { 40.0 }))
			.collect();
		let (start, end, top) = visible_range(&rows, 1000.0, 1500.0);
		assert!(start > 0);
		assert!(end - start < 12);
		assert!(top <= 1000.0);
		assert_eq!(visible_range(&[], 0.0, 100.0), (0, 0, 0.0));
		let neighbors = [(Id(1), 40.0), (Id(3), 100.0), (Id(4), 60.0)];
		assert_eq!(visible_range(&neighbors, 40.0, 100.0), (1, 2, 40.0));
		assert_eq!(anchor_offset(&neighbors, Id(2), 25.0), 40.0);
		assert_eq!(anchor_offset(&neighbors, Id(5), 25.0), 140.0);
		assert_eq!(anchor_offset(&neighbors, Id(3), 25.0), 65.0);
		assert_eq!(anchor_offset(&neighbors, Id(3), -25.0), 15.0);
		assert_eq!(anchor_offset(&neighbors, Id(3), -100.0), 0.0);
		assert_eq!(anchor_offset(&neighbors, Id(3), 200.0), 140.0);
		assert_eq!(anchor_offset(&[], Id(2), 25.0), 0.0);
	}

	fn overscan_frame(
		ctx: &egui::Context,
		view: &mut TimelineView,
		state: &mut State,
		avatars: &mut crate::avatars::Avatars,
		width: f32,
		events: Vec<egui::Event>,
	) {
		ctx.run_ui(
			egui::RawInput {
				focused: true,
				events,
				screen_rect: Some(egui::Rect::from_min_size(
					egui::Pos2::ZERO,
					egui::vec2(width, 600.0),
				)),
				..Default::default()
			},
			|ui| {
				view.show(
					ui,
					state,
					&mut None,
					&mut None,
					(avatars, &mut crate::profiles::ProfileSession::default()),
					None,
				)
			},
		)
		.drop_without_applying_deltas();
	}

	fn overscan_state() -> State {
		let mut state = State {
			selected: Some(Id(20)),
			revision: 1,
			demo: true,
			..Default::default()
		};
		for id in 1..=500 {
			state
				.timeline
				.insert(text_message(id), false, false)
				.unwrap();
		}
		state
	}

	#[test]
	#[ignore = "release performance workload"]
	fn leading_overscan_benchmark() {
		for width in [900.0, 360.0] {
			let ctx = egui::Context::default();
			crate::design::apply(&ctx);
			let mut state = overscan_state();
			let mut view = TimelineView::default();
			let mut avatars = crate::avatars::Avatars::default();
			for _ in 0..10 {
				overscan_frame(&ctx, &mut view, &mut state, &mut avatars, width, vec![]);
			}
			view.following = false;
			view.anchor = Some((Id(250), 5.0));
			view.revision = u64::MAX;
			for _ in 0..10 {
				overscan_frame(&ctx, &mut view, &mut state, &mut avatars, width, vec![]);
			}
			for sample in 0..6 {
				let started = std::time::Instant::now();
				let mut rendered = 0;
				for _ in 0..1000 {
					overscan_frame(&ctx, &mut view, &mut state, &mut avatars, width, vec![]);
					rendered += view.leading_rendered;
				}
				println!(
					"width={width} sample={sample} elapsed_ms={} leading_rendered={rendered}",
					started.elapsed().as_secs_f64() * 1000.0
				);
			}
		}
	}

	#[test]
	fn leading_overscan_reuses_only_current_uninteracted_text_measurements() {
		let ctx = egui::Context::default();
		crate::design::apply(&ctx);
		let mut state = overscan_state();
		let mut view = TimelineView::default();
		let mut avatars = crate::avatars::Avatars::default();
		for _ in 0..10 {
			overscan_frame(&ctx, &mut view, &mut state, &mut avatars, 900.0, vec![]);
		}
		view.following = false;
		view.anchor = Some((Id(250), 5.0));
		view.revision = u64::MAX;
		for width in [900.0, 360.0] {
			overscan_frame(&ctx, &mut view, &mut state, &mut avatars, width, vec![]);
			assert!(
				view.leading_rendered > 0,
				"New dimensions require measurement"
			);
			for _ in 0..10 {
				overscan_frame(&ctx, &mut view, &mut state, &mut avatars, width, vec![]);
			}
			assert_eq!(
				view.leading_rendered, 0,
				"Settled hidden text needs no layout"
			);
		}
		// Pointer selection must retain the complete label registration path.
		for pressed in [true, false] {
			overscan_frame(
				&ctx,
				&mut view,
				&mut state,
				&mut avatars,
				360.0,
				vec![egui::Event::PointerButton {
					pos: egui::pos2(10.0, 10.0),
					button: egui::PointerButton::Primary,
					pressed,
					modifiers: egui::Modifiers::NONE,
				}],
			);
			assert!(view.leading_rendered > 0);
		}
		overscan_frame(&ctx, &mut view, &mut state, &mut avatars, 360.0, vec![]);
		assert!(
			view.leading_rendered > 0,
			"Retained selection or focus still needs registration"
		);
		ctx.plugin::<egui::text_selection::LabelSelectionState>()
			.lock()
			.clear_selection();
		ctx.memory_mut(|memory| {
			if let Some(id) = memory.focused() {
				memory.surrender_focus(id);
			}
		});
		let (anchor, _, _) =
			visible_range(&view.rows, view.scroll_offset, view.scroll_offset + 600.0);
		let id = view.rows[anchor - 1].0;
		let mut message = state.timeline.get(id).unwrap().clone();
		message.content = "A relative timestamp: <t:0:R>".into();
		state.timeline.insert(message, false, false).unwrap();
		state.revision += 1;
		overscan_frame(&ctx, &mut view, &mut state, &mut avatars, 360.0, vec![]);
		assert!(
			view.leading_rendered > 0,
			"State changes invalidate settled heights"
		);
		for _ in 0..10 {
			overscan_frame(&ctx, &mut view, &mut state, &mut avatars, 360.0, vec![]);
		}
		assert_eq!(
			view.leading_rendered, 1,
			"Only the dynamic leading row needs layout"
		);
		let (anchor, _, _) =
			visible_range(&view.rows, view.scroll_offset, view.scroll_offset + 600.0);
		for (id, height) in &view.rows[anchor..] {
			if view.measured_rows.contains(id) {
				assert_eq!(
					*height, view.heights[id].1,
					"Mixed reuse must preserve row alignment"
				);
			}
		}
	}

	#[test]
	fn deleted_only_timeline_keeps_content_without_service_actions() {
		fn texts(shape: &egui::Shape, out: &mut Vec<String>) {
			match shape {
				egui::Shape::Text(text) => out.push(text.galley.job.text.clone()),
				egui::Shape::Vec(shapes) => {
					for shape in shapes {
						texts(shape, out);
					}
				}
				_ => {}
			}
		}
		for (width, dark) in [(240.0, false), (600.0, true)] {
			let mut state = test_support::demo_state();
			state.read_state.reset();
			state.timeline.clear();
			let mut message = text_message(42);
			message.channel = state.selected.unwrap();
			message.author.name = "Deleted synthetic author".into();
			message.content = "||Deleted synthetic body||".into();
			state
				.timeline
				.insert(message.clone(), false, false)
				.unwrap();
			let ctx = egui::Context::default();
			ctx.set_visuals(if dark {
				egui::Visuals::dark()
			} else {
				egui::Visuals::light()
			});
			let mut view = TimelineView::default();
			let mut avatars = crate::avatars::Avatars::default();
			let mut render = |view: &mut TimelineView, state: &mut State| {
				let output = ctx.run_ui(
					egui::RawInput {
						screen_rect: Some(egui::Rect::from_min_size(
							egui::Pos2::ZERO,
							egui::vec2(width, 480.0),
						)),
						..Default::default()
					},
					|ui| {
						view.show(
							ui,
							state,
							&mut None,
							&mut None,
							(
								&mut avatars,
								&mut crate::profiles::ProfileSession::default(),
							),
							None,
						);
						assert!(ui.min_rect().right() <= ui.max_rect().right() + 1.0);
					},
				);
				assert!(
					output
						.platform_output
						.commands
						.iter()
						.all(|command| matches!(
							command,
							egui::OutputCommand::TextSelectionSettled(_)
						))
				);
				let mut labels = vec![];
				for shape in &output.shapes {
					texts(&shape.shape, &mut labels);
				}
				output.drop_without_applying_deltas();
				labels
			};
			for _ in 0..3 {
				render(&mut view, &mut state);
			}
			view.revealed
				.insert(message.id, Revealed::new(&message, u32::MAX, true));
			view.viewing = Some((message.id, Id(9)));
			view.toolbar = Some((message.id, egui::Rect::EVERYTHING));
			state.set_preserve_deleted_messages(true);
			state.timeline.delete(message.id).unwrap();
			state.revision += 1;
			for _ in 0..3 {
				render(&mut view, &mut state);
			}
			let labels = render(&mut view, &mut state);
			assert!(state.timeline.is_empty());
			assert_eq!(state.timeline.row_count(), 1);
			assert!(state.timeline.get_display(message.id).is_some());
			assert!(!view.rows.is_empty());
			assert!(
				labels
					.iter()
					.any(|label| label.contains("Deleted synthetic author"))
			);
			assert!(
				labels
					.iter()
					.any(|label| label.contains("Deleted synthetic body"))
			);
			for text in ["Reply", "Open in Discord"] {
				assert!(
					!labels.iter().any(|label| label.contains(text)),
					"Deleted row exposed service action {text}"
				);
			}
			assert!(view.reaction.is_none());
			let selected = state.channel(state.selected.unwrap()).unwrap();
			assert_ne!(selected.last_message, Some(message.id));
			assert_eq!(view.mark_read, selected.last_message);
		}
	}
	#[test]
	fn channel_rename_invalidates_offscreen_reference_heights() {
		check_channel_rename_heights();
	}

	fn check_channel_rename_heights() {
		let message = Message {
			poll: None,
			sticker_items: vec![],
			id: Id(1),
			channel: Id(2),
			author: model::User {
				id: Id(3),
				name: "Synthetic".into(),
				avatar: None,
				webhook: false,
				kind: Default::default(),
				discriminator: 0,
				primary_guild: None,
			},
			content: "<#4> ".repeat(12),
			author_nick: None,
			author_roles: vec![],
			mention_roles: vec![],
			mention_everyone: false,
			suppress_notifications: false,
			mentions: vec![],
			reactions: Some(vec![]),
			edited: false,
			edited_at: None,
			revision: 0,
			nonce: None,
			reply_to: None,
			kind: 0,
			reply_deleted: false,
			interaction: None,
			forwarded: false,
			unsupported: false,
			extra_content: Default::default(),
			components: vec![],
			application_id: None,
			ephemeral: false,
			flags: 0,
			embeds: vec![],
			embeds_suppressed: false,
			attachments: vec![],
		};
		let message_key = layout_key(&message);
		let mut tail = message.clone();
		tail.id = Id(2);
		tail.content = "ordinary text ".repeat(350);
		let mut state = State {
			demo: true,
			selected: Some(Id(2)),
			revision: 1,
			..Default::default()
		};
		state.channels.push(model::Channel {
			id: Id(4),
			guild: Some(Id(5)),
			name: "a".into(),
			kind: 0,
			parent_id: None,
			position: 0,
			recipients: vec![],
			member_list_id: None,
			tags: None,
			message_count: None,
			icon: None,
			last_message: None,
		});
		state.timeline.insert(message, false, false).unwrap();
		state.timeline.insert(tail, false, false).unwrap();
		let mut view = TimelineView {
			channel: state.selected,
			anchor: Some((Id(1), 0.0)),
			..Default::default()
		};
		let mut images = crate::avatars::Avatars::default();
		let context = egui::Context::default();
		let render =
			|view: &mut TimelineView, state: &mut State, images: &mut crate::avatars::Avatars| {
				context
					.run_ui(
						egui::RawInput {
							screen_rect: Some(egui::Rect::from_min_size(
								egui::Pos2::ZERO,
								egui::vec2(360.0, 300.0),
							)),
							..Default::default()
						},
						|ui| {
							view.show(
								ui,
								state,
								&mut None,
								&mut None,
								(images, &mut crate::profiles::ProfileSession::default()),
								None,
							)
						},
					)
					.drop_without_applying_deltas();
			};
		for _ in 0..3 {
			render(&mut view, &mut state, &mut images);
		}
		let short_height = view.heights[&Id(1)].1;
		state.apply(client_core::Envelope {
			generation: state.generation,
			event: client_core::Event::Message(test_support::message(1_000_000, Id(4))),
		});
		render(&mut view, &mut state, &mut images);
		assert_eq!(view.heights[&Id(1)].1, short_height);
		view.following = false;
		view.anchor = Some((Id(2), 400.0));
		view.revision = u64::MAX;
		for _ in 0..3 {
			render(&mut view, &mut state, &mut images);
		}
		assert_eq!(view.heights[&Id(1)].1, short_height);
		assert_eq!(view.anchor.unwrap().0, Id(2));
		state.apply(client_core::Envelope {
			generation: state.generation,
			event: client_core::Event::ChannelChanged(model::ChannelPatch {
				id: Id(4),
				name: model::Patch::Value("a-much-longer-channel-reference".into()),
				last_message: model::Patch::Absent,
				parent_id: model::Patch::Absent,
				position: model::Patch::Absent,
				kind: model::Patch::Absent,
				message_count: model::Patch::Absent,
				tags: model::Patch::Absent,
				icon: model::Patch::Absent,
			}),
		});
		assert_eq!(layout_key(state.timeline.get(Id(1)).unwrap()), message_key);
		render(&mut view, &mut state, &mut images);
		assert!(
			!view.heights.contains_key(&Id(1)),
			"An offscreen row must lose its old label-dependent height even though its message did not change"
		);
		view.following = false;
		view.anchor = Some((Id(1), 0.0));
		view.revision = u64::MAX;
		for _ in 0..3 {
			render(&mut view, &mut state, &mut images);
		}
		assert!(
			view.heights[&Id(1)].1 > short_height + 20.0,
			"The renamed references must be measured with their new wrapped labels"
		);
		assert!(images.take_requests().is_empty());
	}

	#[test]
	fn same_id_revision_reset_does_not_reuse_reveal_or_height() {
		let mut message = Message {
			poll: None,
			sticker_items: vec![],
			reactions: Some(vec![]),
			id: Id(1),
			channel: Id(2),
			author: model::User {
				id: Id(3),
				name: "Synthetic".into(),
				avatar: None,
				webhook: false,
				kind: Default::default(),
				discriminator: 0,
				primary_guild: None,
			},
			content: "||old revealed content||".into(),
			edited: false,
			edited_at: None,
			revision: 0,
			nonce: None,
			reply_to: None,
			kind: 0,
			reply_deleted: false,
			interaction: None,
			forwarded: false,
			unsupported: false,
			extra_content: Default::default(),
			components: vec![],
			application_id: None,
			ephemeral: false,
			flags: 0,
			embeds: vec![],
			attachments: vec![],
			author_nick: None,
			author_roles: vec![],
			mention_roles: vec![],
			mention_everyone: false,
			suppress_notifications: false,
			mentions: Vec::new(),
			embeds_suppressed: false,
		};
		let mut view = TimelineView {
			channel: Some(Id(2)),
			..Default::default()
		};
		view.revealed
			.insert(message.id, Revealed::new(&message, u32::MAX, true));
		view.heights
			.insert(message.id, (layout_key(&message), 4000.0));
		message.content = "||new concealed content||".into();
		let current_key = layout_key(&message);
		assert_ne!(view.heights[&message.id].0, current_key);
		let mut state = State {
			selected: Some(Id(2)),
			revision: 1,
			..Default::default()
		};
		state.timeline.insert(message, false, false).unwrap();
		let context = egui::Context::default();
		// Match dimensions so this specifically exercises content invalidation, not resize.
		let output = context.run_ui(Default::default(), |ui| {
			view.width = ui.available_width();
			view.text_size = egui::TextStyle::Body.resolve(ui.style()).size;
			view.scale = ui.ctx().pixels_per_point();
			view.show(
				ui,
				&mut state,
				&mut None,
				&mut None,
				(
					&mut crate::avatars::Avatars::default(),
					&mut crate::profiles::ProfileSession::default(),
				),
				None,
			);
		});
		output.drop_without_applying_deltas();
		assert!(view.revealed.is_empty());
		assert_eq!(
			view.heights[&Id(1)].0,
			row_key(state.timeline.get(Id(1)).unwrap(), None, None, &state)
		);
		assert!(
			view.heights[&Id(1)].1 < 210.0,
			"A short concealed message must keep a compact row even in an unbounded scroll layout: {}",
			view.heights[&Id(1)].1
		);
	}
	#[test]
	fn embed_cards_conceal_spoilers_and_invalidate_reveals_on_embed_only_edits() {
		fn painted_text(shape: &egui::Shape, text: &mut String) {
			match shape {
				egui::Shape::Text(value) => text.push_str(&value.galley.job.text),
				egui::Shape::Vec(shapes) => {
					for shape in shapes {
						painted_text(shape, text);
					}
				}
				_ => {}
			}
		}
		let mut message = Message {
			poll: None,
			sticker_items: vec![],
			reactions: Some(vec![]),
			id: Id(1),
			channel: Id(2),
			author: model::User {
				id: Id(3),
				name: "Synthetic".into(),
				avatar: None,
				webhook: false,
				kind: Default::default(),
				discriminator: 0,
				primary_guild: None,
			},
			content: "Ordinary text".into(),
			edited: false,
			edited_at: None,
			revision: 0,
			nonce: None,
			reply_to: None,
			kind: 0,
			reply_deleted: false,
			interaction: None,
			forwarded: false,
			unsupported: false,
			extra_content: Default::default(),
			components: vec![],
			application_id: None,
			ephemeral: false,
			flags: 0,
			attachments: vec![],
			author_nick: None,
			author_roles: vec![],
			mention_roles: vec![],
			mention_everyone: false,
			suppress_notifications: false,
			mentions: Vec::new(),
			embeds_suppressed: false,
			embeds: vec![model::Embed {
				kind: "rich".into(),
				title: Some("||Hidden title||".into()),
				description: Some("Embed description".into()),
				image: Some(model::EmbedMedia {
					url: Some("https://example.com/image.png".into()),
					width: 320,
					height: 120,
					..Default::default()
				}),
				..Default::default()
			}],
		};
		// A card near the viewport bottom retains its natural height.
		let ctx = egui::Context::default();
		let mut output = ctx.run_ui(
			egui::RawInput {
				screen_rect: Some(egui::Rect::from_min_size(
					egui::Pos2::ZERO,
					egui::vec2(480.0, 80.0),
				)),
				..Default::default()
			},
			|ui| {
				let mut profile = crate::profiles::ProfileSession::default();
				let _ = super::super::embeds::show(
					ui,
					&message,
					&mut FormatCache::default(),
					&mut crate::avatars::Avatars::default(),
					&mut None,
					&mut crate::attachments::DownloadUi::default(),
					&mut profile,
					&State {
						demo: true,
						..Default::default()
					},
					crate::design::MessageCardSurface::Conversation,
				);
				assert!(
					ui.min_rect().height() > 120.0,
					"card must fit its full image plus text even in an 80 pt viewport"
				);
			},
		);
		output.textures_delta.clear();
		let mut state = State {
			demo: true,
			selected: Some(Id(2)),
			..Default::default()
		};
		state
			.timeline
			.insert(message.clone(), false, false)
			.unwrap();
		let mut view = TimelineView {
			channel: state.selected,
			..Default::default()
		};
		let mut images = crate::avatars::Avatars::default();
		let ctx = egui::Context::default();
		let render =
			|view: &mut TimelineView, state: &mut State, images: &mut crate::avatars::Avatars| {
				let output = ctx.run_ui(
					egui::RawInput {
						screen_rect: Some(egui::Rect::from_min_size(
							egui::Pos2::ZERO,
							egui::vec2(900.0, 900.0),
						)),
						..Default::default()
					},
					|ui| {
						view.show(
							ui,
							state,
							&mut None,
							&mut None,
							(images, &mut crate::profiles::ProfileSession::default()),
							None,
						)
					},
				);
				assert!(
					output.platform_output.commands.is_empty(),
					"Rendering must not open external links"
				);
				let mut text = String::new();
				for shape in &output.shapes {
					painted_text(&shape.shape, &mut text);
				}
				output.drop_without_applying_deltas();
				text
			};
		render(&mut view, &mut state, &mut images);
		let concealed = render(&mut view, &mut state, &mut images);
		assert!(concealed.contains("Reveal spoiler"));
		assert!(!concealed.contains("Hidden title"));
		assert!(!concealed.contains("Embed description"));
		view.revealed
			.insert(message.id, Revealed::new(&message, u32::MAX, true));
		let revealed = render(&mut view, &mut state, &mut images);
		assert!(revealed.contains("Hidden title"));
		assert!(revealed.contains("Embed description"));
		assert!(
			images.take_requests().is_empty(),
			"Demo images never enqueue network requests"
		);
		let previous_key = layout_key(&message);
		message.embeds[0].title = Some("||Changed secret||".into());
		assert_ne!(previous_key, layout_key(&message));
		state.timeline.insert(message.clone(), true, false).unwrap();
		state.revision += 1;
		let changed = render(&mut view, &mut state, &mut images);
		assert!(view.revealed.is_empty());
		assert!(!changed.contains("Changed secret"));
		message.embeds[0].title = Some("Visible title".into());
		message.embeds_suppressed = true;
		state.timeline.insert(message.clone(), true, false).unwrap();
		state.revision += 1;
		let suppressed = render(&mut view, &mut state, &mut images);
		assert!(!suppressed.contains("Visible title"));
		assert!(!suppressed.contains("Embed description"));
		// Attachments are independent of SUPPRESS_EMBEDS, but never of spoiler consent.
		message.embeds.clear();
		message.content.clear();
		message.attachments = vec![model::Attachment {
			duration_ms: None,
			waveform: Vec::new(),
			id: Id(7),
			filename: "SPOILER_hidden.png".into(),
			description: None,
			content_type: Some("image/png".into()),
			size: 100,
			spoiler: true,
			media: model::EmbedMedia {
				url: Some("https://cdn.discordapp.com/attachments/2/7/hidden.png".into()),
				width: 320,
				height: 120,
				..Default::default()
			},
		}];
		assert!(message.attachments[0].is_image());
		state.demo = false; // Only collects image request keys; there is no network worker in this test.
		state.timeline.insert(message.clone(), true, false).unwrap();
		state.revision += 1;
		view.viewing = Some((message.id, Id(7)));
		let hidden = render(&mut view, &mut state, &mut images);
		assert!(!hidden.contains("SPOILER_hidden.png"));
		assert!(view.viewing.is_none());
		assert!(
			images
				.take_requests()
				.iter()
				.all(|key| !key.starts_with("media:")),
			"Hidden attachments must not request media; the visible author avatar is independent"
		);
		view.revealed
			.insert(message.id, Revealed::new(&message, u32::MAX, true));
		view.viewing = Some((message.id, Id(7)));
		render(&mut view, &mut state, &mut images); // Modal sizing pass precedes visible paint.
		let shown = render(&mut view, &mut state, &mut images);
		assert!(shown.contains("SPOILER_hidden.png"));
		assert!(shown.contains("Open in browser"));
		let requests = images.take_requests();
		assert!(!requests.is_empty() && requests.iter().all(|key| key.starts_with("media:")));
		assert!(requests.iter().any(|key| key.contains(":320x120:")));
		let previous_key = layout_key(&message);
		message.attachments[0].description = Some("Changed attachment".into());
		assert_ne!(previous_key, layout_key(&message));
		state.timeline.insert(message, true, false).unwrap();
		state.revision += 1;
		let hidden_again = render(&mut view, &mut state, &mut images);
		assert!(view.viewing.is_none() && view.revealed.is_empty());
		assert!(!hidden_again.contains("SPOILER_hidden.png"));
		assert!(images.take_requests().is_empty());
	}

	fn label_ys(output: &egui::FullOutput) -> BTreeMap<String, f32> {
		fn walk(shape: &egui::Shape, out: &mut BTreeMap<String, f32>) {
			match shape {
				egui::Shape::Text(text) => {
					out.insert(text.galley.job.text.clone(), text.pos.y);
				}
				egui::Shape::Vec(shapes) => {
					for shape in shapes {
						walk(shape, out);
					}
				}
				_ => {}
			}
		}
		let mut labels = BTreeMap::new();
		for shape in &output.shapes {
			if shape.clip_rect.is_positive() {
				walk(&shape.shape, &mut labels);
			}
		}
		labels
	}

	#[test]
	fn successive_deletes_keep_the_floor() {
		let ctx = egui::Context::default();
		crate::design::apply(&ctx);
		let mut state = channel_messages(20, 36);
		let boundary = 86_400_000_u64 << 22;
		let first = boundary - 24;
		state.timeline.clear();
		for (offset, id) in (first..first + 36).enumerate() {
			let mut message = text_message(id);
			message.channel = Id(20);
			message.content = format!("Row {}", offset + 1);
			state.timeline.insert(message, false, false).unwrap();
		}
		state.channels[0].last_message = Some(Id(first + 35));
		state.set_preserve_deleted_messages(true);
		let mut view = TimelineView::default();
		let mut frame = 0u32;
		let mut paint = |view: &mut TimelineView, state: &mut State| {
			frame += 1;
			paint_timeline(&ctx, view, state, frame)
		};
		let mut labels = BTreeMap::new();
		for _ in 0..6 {
			labels = paint(&mut view, &mut state);
		}
		let before: f32 = view.rows.iter().map(|(_, height)| height).sum();
		let before_y = labels["Row 36"];
		for id in first..first + 24 {
			state.timeline.delete(Id(id)).unwrap();
			state.revision += 1;
			labels = paint(&mut view, &mut state);
		}
		for _ in 0..4 {
			labels = paint(&mut view, &mut state);
		}
		let after: f32 = view.rows.iter().map(|(_, height)| height).sum();
		let end_y = labels["Row 36"];
		assert!(
			view.following && (after - before).abs() < 48.0 && (end_y - before_y).abs() < 24.0,
			"successive deletes moved the floor: y={before_y:.1}->{end_y:.1} content={before:.1}->{after:.1} offset={:.1}",
			view.scroll_offset
		);
		let previous = state.timeline.get_display(Id(boundary - 1)).unwrap();
		let successor = state.timeline.get_display(Id(boundary)).unwrap();
		assert!(!grouped(Some(previous), successor, None));
		view.following = false;
		view.jump = false;
		view.anchor = Some((Id(first), 0.0));
		view.revision = u64::MAX;
		for _ in 0..4 {
			labels = paint(&mut view, &mut state);
		}
		let first_day = labels
			.keys()
			.filter(|text| text.contains("January 1,"))
			.count();
		assert_eq!(first_day, 1, "successive deletes opened extra day headers");
		view.anchor = Some((Id(boundary), 0.0));
		view.revision = u64::MAX;
		for _ in 0..4 {
			labels = paint(&mut view, &mut state);
		}
		let next_day = labels
			.keys()
			.filter(|text| text.contains("January 2,"))
			.count();
		assert_eq!(next_day, 1, "successive deletes lost the day boundary");
	}

	/// Idle frames at the live edge. A moving label is a bounce the reader can see.
	#[test]
	fn idle_bottom_message_positions_stay_put() {
		for (label, count) in [("long", 80_u64), ("short", 4)] {
			let mut state = State {
				selected: Some(Id(20)),
				revision: 1,
				demo: true,
				older_exhausted: true,
				freshness: model::Freshness::Fresh,
				..Default::default()
			};
			for id in 1..=count {
				let mut message = text_message(id);
				message.content = format!(
					"Row {id}: https://example.com/{id} synthetic idle bottom {}",
					"extra wrapping text ".repeat(if id % 3 == 0 { 10 } else { 0 })
				);
				state.timeline.insert(message, false, false).unwrap();
			}
			let ctx = egui::Context::default();
			crate::design::apply(&ctx);
			let mut view = TimelineView::default();
			let mut avatars = crate::avatars::Avatars::default();
			let mut prev: Option<BTreeMap<String, f32>> = None;
			let mut moves = Vec::new();
			for frame in 0..24 {
				if frame == 16 {
					state.revision += 1;
				}
				let pointer = if frame >= 20 {
					vec![egui::Event::PointerMoved(egui::pos2(180.0, 520.0))]
				} else {
					vec![]
				};
				let output = ctx.run_ui(
					egui::RawInput {
						screen_rect: Some(egui::Rect::from_min_size(
							egui::Pos2::ZERO,
							egui::vec2(900.0, 600.0),
						)),
						time: Some(f64::from(frame) / 60.0),
						focused: true,
						events: pointer,
						..Default::default()
					},
					|ui| {
						view.show(
							ui,
							&mut state,
							&mut None,
							&mut None,
							(
								&mut avatars,
								&mut crate::profiles::ProfileSession::default(),
							),
							None,
						);
					},
				);
				let labels = label_ys(&output);
				let delay = output
					.viewport_output
					.values()
					.next()
					.map(|viewport| viewport.repaint_delay.as_secs_f32());
				if let Some(prev) = &prev {
					let mut worst = 0.0_f32;
					let mut sample = String::new();
					for (text, y) in &labels {
						if !text.starts_with("Row ") || !(80.0..560.0).contains(y) {
							continue;
						}
						if let Some(before) = prev.get(text) {
							let delta = (y - before).abs();
							if delta > worst {
								worst = delta;
								sample = format!("{text} {before:.2}->{y:.2}");
							}
						}
					}
					if worst > 0.5 {
						moves.push(format!(
							"f{frame} {worst:.2}px {sample} off={:.2} follow={} jump={} reflow={} consec={} repaint={delay:?}",
							view.scroll_offset,
							view.following,
							view.jump,
							view.reflow_frames,
							view.consecutive_reflows,
						));
					}
				}
				prev = Some(labels);
				output.drop_without_applying_deltas();
			}
			println!("{label} moves={}", moves.len());
			for line in &moves {
				println!("  {line}");
			}
			let late: Vec<_> = moves
				.iter()
				.filter(|line| {
					let frame: i32 = line
						.trim_start_matches('f')
						.split_whitespace()
						.next()
						.unwrap_or("0")
						.parse()
						.unwrap_or(0);
					frame >= 1
				})
				.cloned()
				.collect();
			assert!(late.is_empty(), "{label} moved after settle: {late:?}");
		}
	}

	fn channel_messages(channel: u64, count: u64) -> State {
		let mut state = State {
			selected: Some(Id(channel)),
			revision: 1,
			demo: true,
			older_exhausted: true,
			freshness: model::Freshness::Fresh,
			channels: vec![model::Channel {
				id: Id(channel),
				guild: None,
				parent_id: None,
				position: 0,
				name: "general".into(),
				kind: 1,
				recipients: vec![],
				member_list_id: None,
				tags: None,
				message_count: None,
				icon: None,
				last_message: (count > 0).then_some(Id(count)),
			}],
			..Default::default()
		};
		for id in 1..=count {
			let mut message = text_message(id);
			message.channel = Id(channel);
			message.content = format!("Row {id}");
			state.timeline.insert(message, false, false).unwrap();
		}
		state
	}

	fn paint_timeline(
		ctx: &egui::Context,
		view: &mut TimelineView,
		state: &mut State,
		frame: u32,
	) -> BTreeMap<String, f32> {
		let mut avatars = crate::avatars::Avatars::default();
		let output = ctx.run_ui(
			egui::RawInput {
				screen_rect: Some(egui::Rect::from_min_size(
					egui::Pos2::ZERO,
					egui::vec2(900.0, 600.0),
				)),
				time: Some(f64::from(frame) / 60.0),
				focused: true,
				..Default::default()
			},
			|ui| {
				view.show(
					ui,
					state,
					&mut None,
					&mut None,
					(
						&mut avatars,
						&mut crate::profiles::ProfileSession::default(),
					),
					None,
				);
			},
		);
		let labels = label_ys(&output);
		output.drop_without_applying_deltas();
		labels
	}

	fn newest_y(labels: &BTreeMap<String, f32>, count: u64) -> Option<f32> {
		labels.get(&format!("Row {count}")).copied()
	}

	#[test]
	fn arriving_messages_keep_the_live_edge_in_view() {
		let ctx = egui::Context::default();
		crate::design::apply(&ctx);
		let mut state = channel_messages(21, 48);
		let mut view = TimelineView::default();
		let mut settled = None;
		for frame in 0..8 {
			settled = newest_y(&paint_timeline(&ctx, &mut view, &mut state, frame), 48);
		}
		let settled = settled.expect("settled tail");
		assert!(view.following, "settled view follows the live edge");
		let arrival = |id: u64| {
			let mut message = text_message(id);
			message.channel = Id(21);
			message.author.id = Id(id);
			message.content = format!("Row {id}\nsecond line\nthird line");
			message
		};
		// Ordinary message: appended to the shared timeline.
		state.timeline.insert(arrival(49), false, false).unwrap();
		state.channels[0].last_message = Some(Id(49));
		state.revision += 1;
		let row_y = |labels: &BTreeMap<String, f32>, id: u64| {
			let prefix = format!("Row {id}\n");
			labels
				.iter()
				.find(|(text, _)| text.starts_with(&prefix))
				.map(|(_, y)| *y)
		};
		let mut seen = Vec::new();
		for frame in 0..8 {
			seen.push(row_y(
				&paint_timeline(&ctx, &mut view, &mut state, 20 + frame),
				49,
			));
		}
		// Three lines of text end where the single settled line ended.
		assert!(
			seen.last()
				.is_some_and(|y| y.is_some_and(|y| y <= settled + 1.0 && y > settled - 80.0)),
			"appended message did not take the live edge: {seen:?} settled={settled} offset={} following={}",
			view.scroll_offset,
			view.following
		);
		// Private command response: merged from the interaction list, with its own footer.
		let mut private = arrival(50);
		private.ephemeral = true;
		private.flags = 64;
		private.interaction = Some(Box::new(model::Interaction {
			user: private.author.clone(),
			command: "ping".into(),
		}));
		state.interactions.ephemeral.push(private);
		state.revision += 1;
		let mut seen = Vec::new();
		for frame in 0..8 {
			seen.push(row_y(
				&paint_timeline(&ctx, &mut view, &mut state, 40 + frame),
				50,
			));
		}
		assert!(
			seen.last()
				.is_some_and(|y| y.is_some_and(|y| y < settled && y > settled - 120.0)),
			"private response did not take the live edge: {seen:?} settled={settled} offset={} following={}",
			view.scroll_offset,
			view.following
		);
		assert!(
			paint_timeline(&ctx, &mut view, &mut state, 60)
				.keys()
				.any(|text| text.starts_with("Only you can see this")),
			"private footer missing"
		);

		{
			let ctx = egui::Context::default();
			crate::design::apply(&ctx);
			let mut state = channel_messages(21, 48);
			let mut view = TimelineView::default();
			let mut settled = None;
			for frame in 0..6 {
				settled = newest_y(&paint_timeline(&ctx, &mut view, &mut state, frame), 48);
			}
			let settled = settled.expect("settled tail");

			view.heights.retain(|id, _| id.0 <= 12);
			view.following = false;
			view.target_browsing = true;
			view.jump = false;
			view.anchor = Some((Id(1), 0.0));
			view.revision = u64::MAX;
			paint_timeline(&ctx, &mut view, &mut state, 20);
			view.follow_latest(&state);
			let mut ack = Vec::new();
			for frame in 0..6 {
				ack.push(newest_y(
					&paint_timeline(&ctx, &mut view, &mut state, 30 + frame),
					48,
				));
			}
			assert!(
				ack.iter()
					.all(|y| y.is_some_and(|y| (y - settled).abs() < 1.0)),
				"ack to the bottom walked the tail {ack:?}, settled at {settled}"
			);

			let mut loading = channel_messages(22, 48);
			loading.timeline.clear();
			loading.freshness = model::Freshness::Loading;
			loading.history_pending = true;
			loading.older_exhausted = false;
			loading.channels[0].last_message = None;
			let mut opened = TimelineView::default();
			paint_timeline(&ctx, &mut opened, &mut loading, 60);
			for id in 1..=48 {
				let mut message = text_message(id);
				message.channel = Id(22);
				message.content = format!("Row {id}");
				loading.timeline.insert(message, false, false).unwrap();
			}
			loading.freshness = model::Freshness::Fresh;
			loading.history_pending = false;
			loading.older_exhausted = true;
			loading.channels[0].last_message = Some(Id(48));
			loading.revision += 1;
			let mut arrived = Vec::new();
			for frame in 0..6 {
				arrived.push(newest_y(
					&paint_timeline(&ctx, &mut opened, &mut loading, 70 + frame),
					48,
				));
			}
			assert!(
				arrived
					.iter()
					.all(|y| y.is_some_and(|y| (y - settled).abs() < 1.0)),
				"opening onto loaded history walked the tail {arrived:?}, settled at {settled}"
			);
		}
	}
}

/// Offline check that the gesture uses its saved preset instead of recommendations.
#[cfg(all(debug_assertions, feature = "demo"))]
impl crate::MessagingUi {
	pub fn debug_double_click_reaction_check(mut state: State, mut message: Message) {
		state.timeline.clear();
		state.read_state.reset();
		state.selected = Some(message.channel);
		message.content = "preset-check".into();
		message.attachments.clear();
		message.embeds.clear();
		message.reactions = Some(vec![]);
		let id = message.id;
		state.user = Some(message.author.clone());
		state.timeline.insert(message, false, false).unwrap();
		let ctx = egui::Context::default();
		crate::design::apply(&ctx);
		let preferences = model::ReadingPreferences {
			double_click_reaction_enabled: true,
			double_click_reaction: 3,
			..Default::default()
		};
		let mut view = TimelineView {
			quick_reactions: Some(["🚀", "👍", "😂"]),
			double_click_reaction: Some(preferences.double_click_emoji()),
			..Default::default()
		};
		let mut avatars = crate::avatars::Avatars::default();
		let mut time = 0.0;
		let mut scroll = crate::scroll::Session::default();
		let modifiers = std::cell::Cell::new(egui::Modifiers::NONE);
		let labels = std::cell::RefCell::new(Vec::<String>::new());
		let menu_id = egui::Id::unique(("message-menu", state.generation, id));
		let mut render = |view: &mut TimelineView, mut events: Vec<egui::Event>| {
			events.insert(0, egui::Event::ModifiersChanged(modifiers.get()));
			time += 1.0 / 60.0;
			let output = ctx.run_ui(
				egui::RawInput {
					screen_rect: Some(egui::Rect::from_min_size(
						egui::Pos2::ZERO,
						egui::vec2(900.0, 600.0),
					)),
					time: Some(time),
					events,
					..Default::default()
				},
				|ui| {
					view.show_with_scroll(
						ui,
						&mut state,
						&mut None,
						&mut None,
						(
							&mut avatars,
							&mut crate::profiles::ProfileSession::default(),
						),
						None,
						&mut scroll,
					);
				},
			);
			let y = output.shapes.iter().find_map(|shape| match &shape.shape {
				egui::Shape::Text(text) if text.galley.text() == "preset-check" => {
					Some(text.pos.y + text.galley.size().y * 0.5)
				}
				_ => None,
			});
			*labels.borrow_mut() = output
				.shapes
				.iter()
				.filter_map(|shape| match &shape.shape {
					egui::Shape::Text(text) => Some(text.galley.text().to_owned()),
					_ => None,
				})
				.collect();
			output.drop_without_applying_deltas();
			y
		};
		for _ in 0..5 {
			render(&mut view, vec![]);
		}
		let pos = egui::pos2(450.0, render(&mut view, vec![]).expect("message laid out"));
		assert!(!model::ReadingPreferences::default().double_click_reaction_enabled);
		for enabled in [false, true] {
			view.double_click_reaction = enabled.then_some(preferences.double_click_emoji());
			for _ in 0..30 {
				render(&mut view, vec![]);
			}
			for _ in 0..2 {
				for pressed in [true, false] {
					render(
						&mut view,
						vec![
							egui::Event::PointerMoved(pos),
							egui::Event::PointerButton {
								pos,
								button: egui::PointerButton::Primary,
								pressed,
								modifiers: egui::Modifiers::NONE,
							},
						],
					);
				}
			}
			if !enabled {
				assert!(
					view.reaction.is_none(),
					"disabled double-click must not react"
				);
			}
		}
		assert_eq!(
			view.reaction,
			Some((
				id,
				Some(model::ReactionEmoji {
					id: None,
					name: Some("🎉".into())
				})
			))
		);
		modifiers.set(egui::Modifiers::SHIFT);
		render(&mut view, vec![egui::Event::PointerMoved(pos)]);
		let more = view.toolbar.expect("Shift toolbar").1.right_center() - egui::vec2(14.0, 0.0);
		render(&mut view, vec![egui::Event::PointerMoved(more)]);
		render(&mut view, vec![]);
		assert!(
			labels
				.borrow()
				.iter()
				.any(|label| label == &crate::i18n::translate("profiles-show-more")),
			"action label appears without waiting for the tooltip delay: {:?} {:?}",
			labels.borrow(),
			view.toolbar
		);
		for pressed in [true, false] {
			render(
				&mut view,
				vec![egui::Event::PointerButton {
					pos: more,
					button: egui::PointerButton::Primary,
					pressed,
					modifiers: egui::Modifiers::SHIFT,
				}],
			);
		}
		assert!(
			egui::Popup::is_id_open(&ctx, menu_id),
			"Shift-click opens More"
		);
		render(&mut view, vec![]);
		assert!(
			egui::Popup::is_id_open(&ctx, menu_id),
			"More stays open with Shift held"
		);
		modifiers.set(egui::Modifiers::NONE);
		render(&mut view, vec![]);
		assert!(
			egui::Popup::is_id_open(&ctx, menu_id),
			"More stays open after releasing Shift"
		);
		println!("Immediate message labels and Shift menu checks passed.");
		println!("Double-click uses the saved reaction independently of recommendations");
	}
}
