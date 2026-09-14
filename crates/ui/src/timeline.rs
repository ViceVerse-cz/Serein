use crate::design::LazyHover;
use crate::markdown::{FormatCache, discord_url};
use client_core::State;
use egui::RichText;
use model::{Id, Message};
use std::{
	collections::BTreeMap,
	hash::{DefaultHasher, Hash, Hasher},
};

#[derive(Default)]
pub struct TimelineView {
	pub(super) extension_actions: std::sync::Arc<Vec<crate::extensions_ui::MenuAction>>,
	pub(super) extension_request: Option<(crate::extensions_ui::MenuAction, String)>,
	pub(super) user_action: Option<crate::user_menu::Action>,
	pub(super) restore_pending: Option<String>,
	pub(super) cancel_upload: bool,
	pending_heights: BTreeMap<String, f32>,
	pub(super) hide_media_links: bool,
	applied_hide_media_links: bool,
	pub(super) gif_favorite: Option<model::Gif>,
	pub(super) invite_requests: Vec<String>,
	pub(super) invite_join: Option<String>,
	pub(super) edit_started: bool,
	pub(super) reply_started: bool,
	pub(super) quick_delete: Option<(Id, Id)>,
	pub(super) channel_reference: Option<Id>,
	pub(super) reply_target: Option<Id>,
	highlighted: Option<(Id, f64)>,
	target_browsing: bool,
	initial_read_checked: bool,
	pub(super) unread_jump: bool,
	pub(super) load_newer: bool,
	channel_labels: u64,
	pub(super) mark_read: Option<Id>,
	auto_read_attempt: Option<Id>,
	at_current_latest: bool,
	pub(super) reaction_picker: Option<(Id, egui::Rect, egui::Id)>,
	pub(super) reaction: Option<(Id, Option<model::ReactionEmoji>)>,
	/// Requested pin change: channel, message, pinned.
	pub(super) pin_request: Option<(Id, Id, bool)>,
	toolbar: Option<(Id, egui::Rect)>,
	heights: BTreeMap<Id, (u64, f32)>,
	pub(super) reflow_frames: u64,
	pub(super) consecutive_reflows: u64,
	width: f32,
	rows: Vec<(Id, f32)>,
	revision: u64,
	channel: Option<Id>,
	anchor: Option<(Id, f32)>,
	following: bool,
	formatted: FormatCache,
	pending_formatted: FormatCache,
	// A fingerprint of the revealed content prevents a reload that resets model revisions from
	// revealing edits, without cloning payloads. Pruned with the active window: at most 500 records.
	revealed: BTreeMap<Id, Revealed>,
	viewing: Option<(Id, Id)>,
	/// Fixture-only: viewer to open once its message has arrived in the timeline.
	pending_viewer: Option<(Id, Id)>,
	pub(super) download: crate::attachments::DownloadUi,
	pub(super) audio: crate::audio::AudioUi,
	pub(super) video: crate::video::VideoUi,
	pub(super) opening: Option<String>,
	text_size: f32,
	scale: f32,
	pub(super) load_older: bool,
	pub(super) latest: bool,
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
		crate::design::semibold(ui, format!("Welcome to #{}", channel.name), 28.0)
			.color(colors.text_strong),
	)
	.into_galley(
		ui,
		Some(egui::TextWrapMode::Wrap),
		width,
		egui::TextStyle::Heading,
	);
	let description = egui::WidgetText::from(
		RichText::new("This is the beginning of the conversation.").color(colors.muted),
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
					10..=12 => crate::icons::Icon::Threads,
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

fn loading_messages(ui: &mut egui::Ui, fill_viewport: bool) {
	let colors = crate::design::palette(ui);
	let height = if fill_viewport {
		ui.available_height()
	} else {
		ui.available_height().min(144.0)
	};
	let (rect, response) = ui.allocate_exact_size(
		egui::vec2(ui.available_width(), height),
		egui::Sense::hover(),
	);
	response.widget_info(|| {
		egui::WidgetInfo::labeled(egui::WidgetType::Label, false, "Loading messages")
	});
	let painter = ui.painter().with_clip_rect(ui.clip_rect().intersect(rect));
	let fill = colors.muted.gamma_multiply(0.22);
	let text_width = (rect.width() - 88.0).clamp(0.0, 480.0);
	let rows = if fill_viewport {
		(height / 68.0).ceil().clamp(0.0, 128.0) as usize
	} else {
		2
	};
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
fn anchor_offset(rows: &[(Id, f32)], id: Id, inset: f32) -> f32 {
	if rows.is_empty() {
		return 0.0;
	}
	// Keep the next surviving message at the top; fall back to the previous one at the end.
	let index = rows
		.partition_point(|(row, _)| *row < id)
		.min(rows.len() - 1);
	let within = if rows[index].0 == id {
		inset.clamp(0.0, rows[index].1.max(0.0))
	} else {
		0.0
	};
	rows[..index].iter().map(|(_, height)| *height).sum::<f32>() + within
}
fn layout_key(message: &Message) -> u64 {
	// A layout fingerprint only; spoiler visibility uses exact text instead.
	let mut key = DefaultHasher::new();
	message.content.hash(&mut key);
	message.reactions.hash(&mut key);
	for user in &message.mentions {
		user.id.hash(&mut key);
		user.name.hash(&mut key);
	}
	message.author.name.hash(&mut key);
	message.author.kind.hash(&mut key);
	message.author.webhook.hash(&mut key);
	message.edited.hash(&mut key);
	message.reply_to.hash(&mut key);
	message.forwarded.hash(&mut key);
	message.reply_deleted.hash(&mut key);
	message.unsupported.hash(&mut key);
	message.extra_content.hash(&mut key);
	message.kind.hash(&mut key);
	message.attachments.hash(&mut key);
	message.embeds.hash(&mut key);
	message.embeds_suppressed.hash(&mut key);
	key.finish()
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
		18 | 21 => (Icon::Threads, colors.muted),
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
fn grouped(previous: Option<&Message>, message: &Message, boundary: Option<Id>) -> bool {
	previous.is_some_and(|previous| {
		previous.author.id == message.author.id
			&& previous.author.account_label() == message.author.account_label()
			&& message.reply_to.is_none()
			&& !message.unsupported
			&& !previous.unsupported
			&& !message.extra_content.any()
			&& !previous.extra_content.any()
			&& boundary != Some(message.id)
			&& timestamp(previous.id).date() == timestamp(message.id).date()
			&& (timestamp(message.id) - timestamp(previous.id)).whole_seconds() < 300
	})
}
fn row_key(message: &Message, previous: Option<&Message>, boundary: Option<Id>) -> u64 {
	let mut key = DefaultHasher::new();
	layout_key(message).hash(&mut key);
	grouped(previous, message, boundary).hash(&mut key);
	previous
		.is_none_or(|p| timestamp(p.id).date() != timestamp(message.id).date())
		.hash(&mut key);
	(boundary == Some(message.id)).hash(&mut key);
	key.finish()
}
fn divider(ui: &mut egui::Ui, label: String, unread: bool) {
	let colors = crate::design::palette(ui);
	let color = if unread { colors.danger } else { colors.muted };
	ui.add_space(16.0);
	ui.horizontal(|ui| {
		ui.add_space(16.0);
		let font = egui::FontId::new(12.0, crate::design::semibold_family(ui.ctx()));
		let text = ui.painter().layout_no_wrap(label.clone(), font, color);
		let (rect, response) = ui.allocate_exact_size(
			egui::vec2((ui.available_width() - 16.0).max(0.0), 20.0),
			egui::Sense::hover(),
		);
		response.widget_info(|| {
			egui::WidgetInfo::labeled(egui::WidgetType::Label, ui.is_enabled(), &label)
		});
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
		ui.painter().galley(
			egui::pos2(
				rect.center().x - text.size().x / 2.0,
				rect.center().y - text.size().y / 2.0,
			),
			text,
			color,
		);
	});
	ui.add_space(4.0);
}
fn action_button(ui: &mut egui::Ui, icon: crate::icons::Icon, label: &str) -> egui::Response {
	crate::icons::button(ui, icon, 28.0, label)
}
fn message_actions(
	popup: egui::Popup<'_>,
	(message, extension_actions, extension_request): (
		&Message,
		&[crate::extensions_ui::MenuAction],
		&mut Option<(crate::extensions_ui::MenuAction, String)>,
	),
	actions: (bool, bool, bool, bool),
	selection: (Option<&mut Option<Id>>, &mut Option<Id>),
	editing: (&mut Option<(Id, Id, String)>, &mut bool),
	deleting: &mut Option<(Id, Id)>,
	pin: (bool, bool, &mut Option<(Id, Id, bool)>),
) {
	let (mark_read, reply) = selection;
	let (editing, edit_started) = editing;
	let (own, can_reply, can_edit, can_delete) = actions;
	let (can_pin, pinned, pin_request) = pin;
	popup.show(|ui| {
		ui.set_min_width(160.0);
		if !extension_actions.is_empty() {
			ui.menu_button("Extensions", |ui| {
				for action in extension_actions {
					if ui.button(&action.label).clicked() {
						*extension_request =
							Some((action.clone(), message.display_text().into_owned()));
						ui.close();
					}
				}
			});
			ui.separator();
		}
		if ui.button("Copy message").clicked() {
			ui.ctx().copy_text(message.display_text().into_owned());
			ui.close();
		}
		if ui
			.add_enabled(can_reply, egui::Button::new("Reply"))
			.clicked()
		{
			*reply = Some(message.id);
			ui.close();
		}
		if ui
			.add_enabled(
				mark_read.is_some(),
				egui::Button::new("Mark read through here"),
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
				can_pin,
				egui::Button::new(if pinned {
					"Unpin message"
				} else {
					"Pin message"
				}),
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
				.add_enabled(can_edit, egui::Button::new("Edit message"))
				.clicked()
		{
			*editing = Some((message.channel, message.id, message.content.clone()));
			*edit_started = true;
			ui.close();
		}
		if (own || can_delete)
			&& ui
				.add_enabled(can_delete, egui::Button::new("Delete message\u{2026}"))
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
			.id(ui.make_persistent_id(("timeline-overlay", radius.sw == 0)))
			.max_rect(rect.shrink2(egui::vec2(12.0, 0.0)))
			.layout(egui::Layout::left_to_right(egui::Align::Center)),
	);
	bar.spacing_mut().item_spacing.x = 8.0;
	add(&mut bar);
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
/// Discord-style system row: muted sentence, strong clickable names, inline timestamp.
fn show_system(
	ui: &mut egui::Ui,
	system: &model::SystemMessage,
	time: time::OffsetDateTime,
	state: &State,
	profile: &mut Option<model::User>,
	user_action: &mut Option<crate::user_menu::Action>,
) {
	{
		let colors = crate::design::palette(ui);
		ui.horizontal_wrapped(|ui| {
			ui.spacing_mut().item_spacing = egui::vec2(0.0, 2.0);
			for segment in &system.segments {
				if !segment.strong {
					ui.label(RichText::new(&segment.text).color(colors.muted));
					continue;
				}
				let text = crate::design::medium(ui, &segment.text, 15.0).color(colors.text_strong);
				let Some(user) = &segment.user else {
					ui.add(egui::Label::new(text));
					continue;
				};
				let response = ui
					.add(egui::Label::new(text).sense(egui::Sense::click()))
					.on_hover_cursor(egui::CursorIcon::PointingHand);
				crate::user_menu::show(&response, state, user, profile, user_action);
				if response.clicked() {
					*profile = Some(user.clone());
				}
			}
			ui.add_space(8.0);
			ui.label(
				RichText::new(format!("{:02}:{:02}", time.hour(), time.minute()))
					.size(12.0)
					.color(colors.muted),
			)
			.on_hover_text_with(|| format!("{time} UTC"));
		});
	}
}
impl TimelineView {
	/// Fixture-only: open the media viewer on one attachment.
	#[cfg(any(test, feature = "demo"))]
	pub(super) fn preview_image_viewer(&mut self, message: Id, attachment: Id) {
		self.pending_viewer = Some((message, attachment));
	}
	pub(super) fn viewing_latest(&self, channel: Id) -> bool {
		self.channel == Some(channel) && self.following && self.at_current_latest
	}
	/// Leaving the latest page is deliberate reading; nothing is acknowledged automatically.
	fn browse_away(&mut self) {
		self.target_browsing = true;
		self.following = false;
		self.jump = false;
		self.mark_read = None;
	}
	pub(super) fn follow_latest(&mut self) {
		self.target_browsing = false;
		self.following = true;
		self.jump = true;
		self.anchor = None;
	}
	pub fn show(
		&mut self,
		ui: &mut egui::Ui,
		state: &mut State,
		editing: &mut Option<(Id, Id, String)>,
		deleting: &mut Option<(Id, Id)>,
		(avatars, profile): (&mut crate::avatars::Avatars, &mut Option<model::User>),
		upload: Option<&crate::pending::Upload>,
	) {
		let width = ui.available_width();
		let channel_changed = self.channel != state.selected;
		if channel_changed {
			*self = Self {
				extension_actions: self.extension_actions.clone(),
				hide_media_links: self.hide_media_links,
				channel: state.selected,
				following: true,
				download: std::mem::take(&mut self.download),
				opening: self.opening.take(),
				pending_viewer: self.pending_viewer.take(),
				jump: true,
				..Self::default()
			};
		}
		let mut initial_unread_gap = false;
		if !self.initial_read_checked
			&& state.freshness == model::Freshness::Fresh
			&& !state.history_pending
			&& let Some(channel) = state.selected
			&& let Some(unread) = state.unread(channel)
		{
			self.initial_read_checked = true;
			let marker_loaded = state
				.read_marker(channel)
				.flatten()
				.is_some_and(|marker| state.timeline.row_ids().any(|id| id == marker));
			if unread && !marker_loaded {
				initial_unread_gap = !self.target_browsing;
				// Opening a recent page is not consent to skip an unseen unread gap.
				self.target_browsing = true;
				self.following = false;
				self.jump = false;
				self.mark_read = None;
			}
		}
		let can_load_newer = state.can_load_newer();
		if self.unread_jump || self.load_newer {
			self.browse_away();
		}
		// Incoming messages being watched at the live edge are not a new unread section.
		// Keep service read state unchanged while its acknowledgement is in flight.
		let watching_latest = self.following
			&& self.at_current_latest
			&& !state.history_targeted
			&& state.history_before.is_none()
			&& state.history_after.is_none()
			&& ui.input(|input| input.focused);
		let boundary = state
			.selected
			.and_then(|channel| state.read_marker(channel))
			.and_then(|read| {
				state
					.timeline
					.iter()
					.find(|m| read.is_none_or(|id| m.id > id))
					.map(|m| m.id)
			})
			.filter(|_| !watching_latest || self.unread_boundary.is_some());
		if self.unread_boundary != boundary {
			self.unread_boundary = boundary;
			self.revision = u64::MAX;
		}
		let text_size = egui::TextStyle::Body.resolve(ui.style()).size;
		let scale = ui.ctx().pixels_per_point();
		let mut labels_changed = false;
		if self.revision != state.revision {
			// ponytail: hash bounded channel labels per state update; use a dedicated
			// navigation revision only if profiling shows this scan is significant.
			let mut labels = DefaultHasher::new();
			for channel in state
				.channels
				.iter()
				.filter(|c| c.guild.is_some() && c.supports_text())
			{
				channel.id.hash(&mut labels);
				channel.guild.hash(&mut labels);
				channel.name.hash(&mut labels);
			}
			let labels = labels.finish();
			labels_changed = self.channel_labels != labels;
			self.channel_labels = labels;
		}
		let dimensions_changed = (self.width - width).abs() > 1.0
			|| self.text_size != text_size
			|| self.scale != scale
			|| labels_changed
			|| self.hide_media_links != self.applied_hide_media_links;
		self.applied_hide_media_links = self.hide_media_links;
		let changed = self.revision != state.revision || dimensions_changed;
		let mut offset = None;
		if changed {
			if dimensions_changed {
				self.heights.clear();
				self.pending_heights.clear();
			}
			self.width = width;
			self.text_size = text_size;
			self.scale = scale;
			let row_ids: Vec<_> = state
				.timeline
				.display_iter()
				.map(|message| message.id)
				.collect();
			self.heights
				.retain(|id, _| row_ids.binary_search(id).is_ok());
			self.formatted.retain(|id| state.timeline.get(id).is_some());
			self.toolbar = self
				.toolbar
				.filter(|(id, _)| state.timeline.get(*id).is_some());
			self.revealed
				.retain(|id, content| state.timeline.get(*id).is_some_and(|m| content.matches(m)));
			let mut previous = None;
			self.rows = state
				.timeline
				.display_iter()
				.map(|m| {
					let key = row_key(m, previous, self.unread_boundary)
						^ u64::from(state.timeline.is_deleted(m.id));
					previous = (!state.timeline.is_deleted(m.id)).then_some(m);
					let estimate = (if m.embeds_suppressed {
						0.0
					} else {
						crate::embeds::estimated_height(&m.embeds)
							+ crate::invites::estimated_height(m)
					}) + crate::attachments::estimated_height(
						&m.attachments,
						(width - 88.0).max(1.0),
					) + 58.0 + 18.0
						* (m.content
							.lines()
							.map(|line| {
								(line.chars().count() as f32 / ((width - 80.0) / 8.0).max(1.0))
									.ceil()
									.max(1.0)
							})
							.sum::<f32>())
						.min(128.0);
					let height = self
						.heights
						.get(&m.id)
						.filter(|(old_key, _)| *old_key == key)
						.map_or(estimate, |(_, height)| *height);
					(m.id, height)
				})
				.collect();
			self.revision = state.revision;
			if !self.following
				&& let Some((id, inset)) = self.anchor
			{
				offset = Some(anchor_offset(&self.rows, id, inset));
			}
		}
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
			ui.weak("Message history is unavailable with current permission information.");
		}
		if history_available && state.freshness == model::Freshness::Loading {
			loading_messages(ui, empty);
			if empty {
				return;
			}
		} else if empty && history_available && !welcome {
			ui.label(match state.freshness {
				model::Freshness::Loading => "Loading messages…",
				model::Freshness::Unavailable => "You cannot view this conversation.",
				model::Freshness::Stale => "History is not available yet. Use Reload to try again.",
				model::Freshness::Fresh => "No messages yet. Start the conversation below.",
			});
		}
		if state.freshness == model::Freshness::Fresh
			&& !state.history_pending
			&& let Some(target) = state.search_target.take()
		{
			initial_unread_gap = false;
			// Target browsing is deliberate reading, even when the service omits the target.
			// A short result page must not acknowledge unrelated newer messages automatically.
			self.target_browsing = true;
			self.mark_read = None;
			if state.timeline.get(target).is_some() {
				self.highlighted = Some((target, ui.input(|input| input.time) + 2.0));
				self.following = false;
				self.jump = false;
				self.anchor = Some((target, 0.0));
				offset = Some(anchor_offset(&self.rows, target, 0.0));
			} else {
				state.status =
					"Message was not returned; it may have been removed or become unavailable";
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
			.stick_to_bottom(self.following);
		let total: f32 = self.rows.iter().map(|(_, height)| height).sum();
		let end_padding = 16.0;
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
		if std::mem::take(&mut self.jump) {
			offset = Some(
				(total + end_padding + pending_rows.iter().map(|(_, height)| height).sum::<f32>()
					- ui.available_height())
				.max(0.0),
			);
		}
		if let Some(offset) = offset {
			scroll = scroll.vertical_scroll_offset(offset);
		}
		let mut measurements = Vec::new();
		let mut selected_reply = None;
		// ScrollArea consumes wheel input while applying it; retain the viewing gesture.
		let scroll_delta = ui.input(|input| input.smooth_scroll_delta().y);
		let allow_hover =
			!ui.input(|input| input.is_scrolling()) && ui.ctx().dragged_id().is_none();
		let output = scroll.show_viewport(ui, |ui, viewport| {
			ui.spacing_mut().item_spacing.y = 0.0;
			if welcome && let Some(channel) = state.selected.and_then(|id| state.channel(id)) {
				channel_welcome(ui, channel, viewport.height() - end_padding);
			}
			let (first, _, top) = visible_range(
				&self.rows,
				(viewport.min.y - 100.0).max(0.0),
				viewport.max.y + 100.0,
			);
			let (anchor, _, anchor_top) = visible_range(&self.rows, viewport.min.y, viewport.max.y);
			let content_top = ui.cursor().top();
			let clip = ui.clip_rect();
			// Measure leading overscan without changing the visible rows or parent bounds.
			// Its new heights take effect together with anchor restoration next pass.
			let mut leading = ui.new_child(egui::UiBuilder::new().id_salt("leading-measurements"));
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
				let (id, _) = &self.rows[index];
				let can_mark_read = state.can_mark_read(*id);
				let Some(message) = state.timeline.get_display(*id) else {
					continue;
				};
				let previous = index
					.checked_sub(1)
					.and_then(|i| state.timeline.get(self.rows[i].0));
				if state.timeline.is_deleted(*id) {
					let colors = crate::design::palette(ui);
					let response = ui.push_id(row_id, |ui| {
						egui::Frame::new()
							.fill(colors.danger.gamma_multiply(0.10))
							.inner_margin(egui::Margin::symmetric(16, 10))
							.show(ui, |ui| {
								ui.set_min_width((width - 32.0).max(1.0));
								ui.horizontal_wrapped(|ui| {
									ui.label(
										RichText::new(&message.author.name)
											.strong()
											.color(colors.danger),
									);
									ui.label(
										RichText::new("Deleted - kept by Message delete protector")
											.small()
											.color(colors.danger),
									);
								});
								ui.add(
									egui::Label::new(
										RichText::new(if message.content.is_empty() {
											"[Deleted message had no text]"
										} else {
											&message.content
										})
										.color(colors.danger),
									)
									.wrap()
									.selectable(true),
								);
							});
					});
					measurements.push((
						*id,
						row_key(message, previous, self.unread_boundary) ^ 1,
						response.response.rect.height(),
					));
					continue;
				}
				let compact = grouped(previous, message, self.unread_boundary);
				let new_day =
					previous.is_none_or(|p| timestamp(p.id).date() != timestamp(*id).date());
				let response = ui.scope_builder(egui::UiBuilder::new().id(row_id), |ui| {
					if new_day {
						let date = timestamp(*id);
						divider(
							ui,
							format!("{} {}, {}", date.month(), date.day(), date.year()),
							false,
						);
					}
					if self.unread_boundary == Some(*id) {
						divider(ui, "New messages".into(), true);
					}
					let colors = crate::design::palette(ui);
					let background = ui.painter().add(egui::Shape::Noop);
					let mut time_rect = None;
					let row = egui::Frame::NONE
						.inner_margin(egui::Margin {
							left: 16,
							right: 16,
							top: if compact { 1 } else { 14 },
							bottom: 1,
						})
						.show(ui, |ui| {
							ui.spacing_mut().item_spacing = egui::vec2(16.0, 4.0);
							if let Some(reply) = message.reply_to {
								ui.horizontal(|ui| {
									ui.spacing_mut().interact_size.y = 18.0;
									ui.spacing_mut().item_spacing.x = 6.0;
									let (gutter, _) = ui.allocate_exact_size(
										egui::vec2(50.0, 18.0),
										egui::Sense::hover(),
									);
									let x = gutter.left() + 20.0;
									let y = gutter.center().y;
									let stroke =
										egui::Stroke::new(2.0, colors.muted.gamma_multiply(0.5));
									ui.painter().line_segment(
										[
											egui::pos2(x, gutter.bottom() + 2.0),
											egui::pos2(x, y + 5.0),
										],
										stroke,
									);
									ui.painter().add(
										egui::epaint::QuadraticBezierShape::from_points_stroke(
											[
												egui::pos2(x, y + 5.0),
												egui::pos2(x, y),
												egui::pos2(x + 5.0, y),
											],
											false,
											egui::Color32::TRANSPARENT,
											stroke,
										),
									);
									ui.painter().line_segment(
										[egui::pos2(x + 5.0, y), egui::pos2(gutter.right(), y)],
										stroke,
									);
									// Reuse only loaded content; never fetch a thread while painting.
									if message.reply_deleted || state.timeline.is_deleted(reply) {
										ui.add(
											egui::Label::new(
												RichText::new("Message deleted")
													.size(13.0)
													.italics()
													.color(colors.muted),
											)
											.truncate(),
										);
									} else {
										ui.add_enabled_ui(
											state.can_open_reply_target(reply),
											|ui| {
												let mut preview = egui::text::LayoutJob::default();
												let text = if let Some(original) =
													state.timeline.get(reply)
												{
													if avatars
														.show(
															ui,
															&original.author,
															16.0,
															state.demo,
														)
														.clicked()
													{
														self.reply_target = Some(reply);
													}
													preview.append(
														&format!("@{}  ", original.author.name),
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
														"Spoiler".into()
													} else {
														original
															.display_text()
															.chars()
															.take(120)
															.map(|c| {
																if matches!(c, '\n' | '\r') {
																	' '
																} else {
																	c
																}
															})
															.collect::<String>()
													}
												} else {
													"Earlier message · View original".into()
												};
												preview.append(
													&text,
													0.0,
													egui::TextFormat {
														font_id: egui::FontId::proportional(13.0),
														color: colors.muted,
														..Default::default()
													},
												);
												if ui
													.add(
														egui::Label::new(preview)
															.truncate()
															.sense(egui::Sense::click()),
													)
													.on_hover_cursor(egui::CursorIcon::PointingHand)
													.on_hover_text("View original message")
													.on_disabled_hover_text(
														"Wait for readable, current message history",
													)
													.clicked()
												{
													self.reply_target = Some(reply);
												}
											},
										);
									}
								});
							}
							let system = message.system_message();
							ui.horizontal_top(|ui| {
								if system.is_some() {
									let (gutter, _) = ui.allocate_exact_size(
										egui::vec2(40.0, 22.0),
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
								} else if compact {
									time_rect = Some(
										ui.allocate_exact_size(
											egui::vec2(40.0, 22.0),
											egui::Sense::hover(),
										)
										.0,
									);
								} else {
									let avatar =
										avatars.show(ui, &message.author, 40.0, state.demo);
									crate::user_menu::show(
										&avatar,
										state,
										&message.author,
										profile,
										&mut self.user_action,
									);
									if avatar.clicked() {
										*profile = Some(message.author.clone());
									}
								}
								ui.vertical(|ui| {
									ui.set_width(ui.available_width());
									if !compact && system.is_none() {
										ui.allocate_ui_with_layout(
											egui::vec2(ui.available_width(), 22.0),
											egui::Layout::left_to_right(egui::Align::Center),
											|ui| {
												ui.spacing_mut().item_spacing.x = 8.0;
												let author = crate::account_badge::name(
													ui,
													&message.author,
													state.user_display_name(&message.author),
													15.5,
													colors.text_strong,
													egui::Sense::click(),
													48.0,
												);
												crate::user_menu::show(
													&author,
													state,
													&message.author,
													profile,
													&mut self.user_action,
												);
												if author.clicked() {
													*profile = Some(message.author.clone());
												}
												let time = timestamp(*id);
												ui.label(
													RichText::new(format!(
														"{:02}:{:02}",
														time.hour(),
														time.minute()
													))
													.size(12.0)
													.color(colors.muted),
												)
												.on_hover_text_with(|| format!("{} UTC", time));
											},
										);
									}
									if let Some(system) = &system {
										show_system(
											ui,
											system,
											timestamp(*id),
											state,
											profile,
											&mut self.user_action,
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
													RichText::new("\u{21aa} Forwarded")
														.size(13.0)
														.italics()
														.color(colors.muted),
												);
												ui.add_space(4.0);
											}
											let formatted =
												self.formatted.get(*id, &message.content);
											let reveal = self
												.revealed
												.get(id)
												.filter(|reveal| reveal.matches(message));
											let before = reveal.map_or((0, false), |reveal| {
												(reveal.text, reveal.media)
											});
											let mut text =
												if formatted.spoilers { before.0 } else { 0 };
											let mut media = before.1;
											let content_shown =
												system.as_ref().is_some_and(|s| s.content_shown);
											if !content_shown
												&& !(self.hide_media_links
													&& crate::embeds::standalone_media_links(
														message,
													)) {
												formatted.show_references(
													ui,
													&mut self.opening,
													&message.mentions,
													profile,
													(
														&state.channels,
														&mut self.channel_reference,
														&state.guilds,
													),
													(avatars, state.demo, &mut text),
												);
											}
											if formatted.limited {
												ui.label(
													RichText::new(
														"Display limited · Copy message for the full text",
													)
													.small()
													.color(colors.muted),
												);
											}
											if crate::embeds::has_media_spoilers(message) && !media
											{
												if ui.button("Reveal spoiler media").clicked() {
													media = true;
												}
											} else {
												crate::invites::show(
													ui,
													message,
													state,
													avatars,
													&mut self.invite_requests,
													&mut self.invite_join,
												);
												if let Some(gif) = crate::embeds::show(
													ui,
													message,
													&mut self.formatted,
													avatars,
													&mut self.opening,
													&mut self.download,
													profile,
													state,
												) {
													self.gif_favorite = Some(gif);
												}
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
												);
											}
											if (text != 0 || media)
												&& ui.small_button("Hide spoilers").clicked()
											{
												text = 0;
												media = false;
											}
											if before != (text, media) {
												if text == 0 && !media {
													self.revealed.remove(id);
												} else {
													self.revealed.insert(
														*id,
														Revealed::new(message, text, media),
													);
												}
												self.heights.remove(id);
												ui.ctx().request_repaint();
											}
											if message.edited {
												ui.label(
													RichText::new("(edited)")
														.small()
														.color(colors.muted),
												);
											}
											let unknown_system = message.unsupported
												&& message.system_summary().is_none();
											if unknown_system || message.extra_content.any() {
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
														message.extra_content.poll,
														"Poll · Preview unavailable",
													),
													(
														message.extra_content.sticker_items
															|| message.extra_content.stickers,
														"Sticker · Preview unavailable",
													),
													(
														message.extra_content.components
															|| message.extra_content.components_v2,
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
												if ui
													.add_enabled(
														target.is_some(),
														egui::Button::new("Open in Discord"),
													)
													.clicked()
												{
													self.opening = target;
												}
											}
										});
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
									if let Some(action) = crate::reactions::show(
										ui,
										state.reactions.display(message),
										state.gateway_connected
											&& state.freshness == model::Freshness::Fresh
											&& state.can_read_history(message.channel),
										state.reactions.busy(),
										(state.history_pending && state.history_before.is_none())
											|| state.reactions.invalidated(message.id),
										(avatars, state.demo),
										|emoji, add| state.can_react(*id, Some(emoji), add),
									) {
										self.reaction = Some((*id, action));
									}
								});
							});
						});
					let rect = row.response.rect;
					let mentioned = state.user.as_ref().is_some_and(|user| {
						message.mentions.iter().any(|mention| mention.id == user.id)
					});
					if mentioned {
						ui.painter().set(
							background,
							egui::Shape::rect_filled(
								rect,
								0.0,
								colors.warning.gamma_multiply(0.10),
							),
						);
						ui.painter().rect_filled(
							egui::Rect::from_min_size(rect.min, egui::vec2(3.0, rect.height())),
							0.0,
							colors.warning,
						);
					}
					let focus = ui.interact(
						rect,
						ui.id().with("message-focus"),
						egui::Sense::focusable_noninteractive(),
					);
					focus.widget_info(|| {
						egui::WidgetInfo::labeled(
							egui::WidgetType::Label,
							true,
							format!(
								"Message by {}. {}Tab for actions.",
								message.author.name,
								if mentioned { "Mentions you. " } else { "" },
							),
						)
					});
					let retained = retained_toolbar.is_some_and(|(active, _)| active == *id);
					// The floating toolbar overlaps the row above; pointer inside it keeps this row active.
					let toolbar_hover = self
						.toolbar
						.filter(|(active, toolbar)| {
							*active == *id && ui.rect_contains_pointer(*toolbar)
						})
						.is_some();
					let other_toolbar_hover = self
						.toolbar
						.filter(|(active, toolbar)| {
							*active != *id && ui.rect_contains_pointer(*toolbar)
						})
						.is_some();
					let hovered = allow_hover
						&& (ui.rect_contains_pointer(rect) || toolbar_hover)
						&& !other_toolbar_hover
						&& !egui::Popup::is_any_open(ui.ctx())
						&& retained_toolbar.is_none_or(|(active, _)| active == *id);
					let context_menu = (ui.rect_contains_pointer(rect) || toolbar_hover)
						&& !other_toolbar_hover
						&& !egui::Popup::is_any_open(ui.ctx())
						&& ui.input(|i| i.pointer.secondary_clicked());
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
								if mentioned {
									colors.warning.gamma_multiply(0.16)
								} else {
									colors.hover.gamma_multiply(0.7)
								},
							),
						);
						if let Some(rect) = time_rect {
							let time = timestamp(*id);
							ui.painter().text(
								rect.center(),
								egui::Align2::CENTER_CENTER,
								format!("{:02}:{:02}", time.hour(), time.minute()),
								egui::FontId::proportional(11.0),
								colors.muted,
							);
							ui.interact(rect, ui.id().with("timestamp"), egui::Sense::hover())
								.on_hover_text_with(|| format!("{} UTC", time));
						}
						let own = state
							.user
							.as_ref()
							.is_some_and(|u| u.id == message.author.id);
						let toolbar_rect = egui::Rect::from_min_size(
							egui::pos2(
								rect.right() - if own { 136.0 } else { 106.0 },
								rect.top() - 10.0,
							),
							egui::vec2(if own { 120.0 } else { 90.0 }, 28.0),
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
						let react = state.can_react(*id, None, true)
							|| message.reactions.as_ref().is_some_and(|items| {
								items
									.iter()
									.any(|r| state.can_react(*id, Some(&r.emoji), true))
							});
						if let Some((anchor, trigger)) = crate::reactions::add_button(
							&mut toolbar,
							react,
							state.reactions.busy(),
						) {
							self.reaction_picker = Some((*id, anchor, trigger));
						}
						let can_reply = state.can_send(message.channel);
						let can_edit = !message.unsupported && state.can_edit(message.channel, *id);
						let can_delete = state.can_delete(message.channel, *id);
						if toolbar
							.add_enabled_ui(can_reply, |ui| {
								action_button(ui, crate::icons::Icon::Reply, "Reply")
							})
							.inner
							.clicked()
						{
							selected_reply = Some(*id);
						}
						if own
							&& toolbar
								.add_enabled_ui(can_edit, |ui| {
									action_button(ui, crate::icons::Icon::Pencil, "Edit message")
								})
								.inner
								.clicked()
						{
							*editing = Some((message.channel, *id, message.content.clone()));
							self.edit_started = true;
						}
						if can_delete
							&& !context_menu && toolbar.input(|input| input.modifiers.shift)
							&& !egui::Popup::is_any_open(toolbar.ctx())
						{
							if toolbar
								.push_id("quick-delete", |ui| {
									action_button(
										ui,
										crate::icons::Icon::Trash,
										"Delete message immediately",
									)
								})
								.inner
								.clicked()
							{
								self.quick_delete = Some((message.channel, *id));
							}
						} else {
							let menu =
								action_button(&mut toolbar, crate::icons::Icon::More, "More");
							menu.widget_info(|| {
								egui::WidgetInfo::labeled(
									egui::WidgetType::Button,
									toolbar.is_enabled(),
									format!("Message actions for {}", message.author.name),
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
							message_actions(
								popup,
								(
									message,
									&self.extension_actions,
									&mut self.extension_request,
								),
								(own, can_reply, can_edit, can_delete),
								(
									can_mark_read.then_some(&mut self.mark_read),
									&mut selected_reply,
								),
								(editing, &mut self.edit_started),
								deleting,
								(
									state.can_pin(message.channel, *id),
									state.is_pinned(message.channel, *id),
									&mut self.pin_request,
								),
							);
						}
						self.toolbar = Some((*id, toolbar_rect));
					}
					if selected_reply.or(state.reply) == Some(*id)
						|| self.highlighted.is_some_and(|(target, _)| target == *id)
					{
						ui.painter().set(
							background,
							egui::Shape::rect_filled(rect, 0.0, colors.accent.gamma_multiply(0.25)),
						);
					}
				});
				measurements.push((
					*id,
					row_key(message, previous, self.unread_boundary),
					response.response.rect.height(),
				));
			}
			let used: f32 = self.rows[..end].iter().map(|(_, height)| *height).sum();
			ui.add_space((total - used).max(0.0));
			for (index, (pending, height)) in pending_rows.iter().enumerate() {
				let compact = index > 0
					|| state.timeline.iter().last().is_some_and(|previous| {
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
						compact,
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
					if !dimensions_changed {
						ui.ctx().request_discard("Pending message height settled");
					}
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
		// ScrollArea applies wheel input after laying out its contents. Preserve that
		// movement when new row measurements rebuild the timeline on the next pass.
		let (anchor, _, anchor_top) = visible_range(
			&self.rows,
			output.state.offset.y,
			output.state.offset.y + output.inner_rect.height(),
		);
		self.anchor = self
			.rows
			.get(anchor)
			.map(|(id, _)| (*id, output.state.offset.y - anchor_top));
		if selected_reply.is_some() {
			state.reply = selected_reply;
			self.reply_started = true;
		}
		let distance_from_bottom =
			(output.content_size.y - output.state.offset.y - output.inner_rect.height()).max(0.0);
		let at_bottom = distance_from_bottom <= 3.0;
		// The live edge counts even when service latest metadata outlived a deleted message;
		// otherwise the unread banners could never resolve for that channel.
		self.at_current_latest = state.live_edge_latest().is_some()
			|| state.timeline.iter().last().is_some_and(|message| {
				state.channels.iter().any(|channel| {
					Some(channel.id) == state.selected && channel.last_message == Some(message.id)
				})
			});

		let whole_conversation_visible =
			state.older_exhausted && output.content_size.y <= output.inner_rect.height() + 3.0;
		if initial_unread_gap
			&& !state.history_targeted
			&& state.history_before.is_none()
			&& state.history_after.is_none()
			&& (whole_conversation_visible || state.timeline.iter().next().is_none())
		{
			self.target_browsing = false;
		}

		if at_bottom
			&& self.at_current_latest
			&& ui.input(|input| {
				(scroll_delta < 0.0
					&& input
						.pointer
						.hover_pos()
						.is_some_and(|pos| output.inner_rect.contains(pos)))
					|| (input.pointer.any_down() && output.state.offset.y > output.inner)
			}) {
			self.target_browsing = false;
			if state.history_targeted || state.history_after.is_some() {
				self.latest = true;
			}
			ui.ctx().request_repaint();
		}
		if self.reply_target.is_some() {
			self.target_browsing = true;
			self.mark_read = None;
		}
		self.following = at_bottom && !self.target_browsing;
		if self.following
			&& !state.history_targeted
			&& state.history_before.is_none()
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
			if self.following {
				self.jump = true;
				// Resizing invalidates heights each frame; settle on the queued repaint
				// instead of paying for an extra layout pass throughout the drag.
				if !dimensions_changed {
					ui.ctx().request_discard("Timeline message heights settled");
				}
			}
			ui.ctx().request_repaint();
		}
		if !reflow {
			self.consecutive_reflows = 0;
		}
		// A user scroll near the top requests one page; a short initial view never drains history.
		self.load_older = !self.following
			&& output.state.offset.y < 160.0
			&& ui.input(|i| {
				i.smooth_scroll_delta().y > 0.0
					&& i.pointer
						.hover_pos()
						.is_some_and(|pos| output.inner_rect.contains(pos))
			}) && state.can_load_older();
		// Discord-style overlays: an unread strip hangs from the top edge, and a translucent
		// "older messages" bar floats above the composer while the user is not following. They
		// are painted after the scroll area so they sit above the messages and win the hit-test.
		let colors = crate::design::palette(ui);
		let area = output.inner_rect;
		let fade_rect = egui::Rect::from_min_max(
			egui::pos2(area.left(), (area.bottom() - 12.0).max(area.top())),
			area.right_bottom(),
		);
		let mut fade = egui::Mesh::default();
		fade.colored_vertex(fade_rect.left_top(), egui::Color32::TRANSPARENT);
		fade.colored_vertex(fade_rect.right_top(), egui::Color32::TRANSPARENT);
		fade.colored_vertex(fade_rect.right_bottom(), colors.chat.gamma_multiply(0.85));
		fade.colored_vertex(fade_rect.left_bottom(), colors.chat.gamma_multiply(0.85));
		fade.add_triangle(0, 1, 2);
		fade.add_triangle(0, 2, 3);
		ui.painter()
			.with_clip_rect(area)
			.add(egui::Shape::mesh(fade));
		let browsing_history = state.history_targeted
			|| state.history_before.is_some()
			|| state.history_after.is_some();
		let can_jump_unread = state.can_jump_unread()
			&& state.timeline.iter().next().is_some()
			&& !(self.following
				&& self.at_current_latest
				&& !browsing_history
				&& ui.input(|input| input.focused));
		// Latest-message metadata can outlive a deleted message. A complete, visible
		// latest page has nowhere useful to jump; targeted pages still need navigation.
		if (can_jump_unread || can_load_newer) && (!whole_conversation_visible || browsing_history)
		{
			let mut jump_unread = false;
			let mut load_newer = false;
			overlay_bar(
				ui,
				egui::Rect::from_min_size(
					egui::pos2(area.left() + 16.0, area.top()),
					egui::vec2((area.width() - 32.0).max(120.0), 28.0),
				),
				colors.accent,
				egui::CornerRadius {
					nw: 0,
					ne: 0,
					sw: 8,
					se: 8,
				},
				|ui| {
					ui.label(
						crate::design::medium(
							ui,
							if can_jump_unread {
								"Unread messages"
							} else {
								"More messages"
							},
							13.0,
						)
						.color(colors.accent_text),
					);
					ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
						if can_jump_unread
							&& bar_button(
								ui,
								"Jump to unread",
								crate::icons::Icon::ArrowUp,
								colors.accent_text,
							)
							.clicked()
						{
							jump_unread = true;
						}
						if can_load_newer
							&& bar_button(
								ui,
								"Next messages",
								crate::icons::Icon::ArrowDown,
								colors.accent_text,
							)
							.clicked()
						{
							load_newer = true;
						}
					});
				},
			);
			if jump_unread {
				self.unread_jump = true;
				self.browse_away();
			}
			if load_newer {
				self.load_newer = true;
				self.browse_away();
			}
		}
		// Older pages appended to the live timeline keep their cursor after loading; that
		// alone must not raise the bar the moment a reader nudges upward. Only pages that are
		// detached from the live edge (targeted or forward history) show it immediately.
		let detached_page = state.history_targeted || state.history_after.is_some();
		if (!self.following && distance_from_bottom > 3.0 * area.height())
			|| self.target_browsing
			|| detached_page
		{
			let unread = state
				.selected
				.is_some_and(|channel| state.unread(channel) == Some(true));
			let mut present = false;
			let rect = egui::Rect::from_min_size(
				egui::pos2(area.left() + 16.0, area.bottom() - 30.0),
				egui::vec2((area.width() - 32.0).max(120.0), 30.0),
			);
			let base = colors.base.to_opaque();
			overlay_bar(
				ui,
				rect,
				egui::Color32::from_rgba_unmultiplied(base.r(), base.g(), base.b(), 236),
				egui::CornerRadius {
					nw: 8,
					ne: 8,
					sw: 0,
					se: 0,
				},
				|ui| {
					ui.label(
						RichText::new(if unread {
							"New messages below"
						} else {
							"You're viewing older messages"
						})
						.size(13.0)
						.color(colors.text),
					);
					ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
						present = bar_button(
							ui,
							"Jump to present",
							crate::icons::Icon::ArrowDown,
							colors.text_strong,
						)
						.clicked();
					});
				},
			);
			if present {
				if browsing_history {
					self.latest = true;
				}
				self.follow_latest();
				ui.ctx().request_repaint();
			}
		}
		if let Some((message_id, attachment_id)) = self.pending_viewer
			&& state.timeline.get(message_id).is_some()
		{
			self.pending_viewer = None;
			self.viewing = Some((message_id, attachment_id));
		}
		if self.video.is_fullscreen() {
			let current = self
				.video
				.active
				.as_ref()
				.and_then(|(channel, id, attachment)| {
					state
						.timeline
						.get(*id)
						.filter(|message| {
							state.selected == Some(*channel)
								&& message.channel == *channel
								&& message.attachments.contains(attachment)
								&& (!crate::embeds::has_media_spoilers(message)
									|| self.revealed.get(id).is_some_and(|reveal| {
										reveal.media && reveal.matches(message)
									}))
						})
						.map(|message| (message, attachment.clone()))
				});
			if let Some((message, attachment)) = current {
				self.video.show_fullscreen(
					ui.ctx(),
					message,
					&attachment,
					&mut self.download,
					&mut self.opening,
					state.demo,
				);
			} else {
				self.video.stop();
			}
		}
		if let Some((message_id, attachment_id)) = self.viewing {
			let message = state.timeline.get(message_id).filter(|m| {
				!crate::embeds::has_media_spoilers(m)
					|| self
						.revealed
						.get(&m.id)
						.is_some_and(|reveal| reveal.media && reveal.matches(m))
			});
			self.viewing = message.and_then(|m| {
				crate::attachments::viewer(
					ui,
					&m.attachments,
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
#[cfg(test)]
#[path = "pending_tests.rs"]
mod pending_tests;
#[cfg(test)]
mod tests {
	use super::*;
	// Synthetic regressions: no transport or acknowledgement worker is running.
	fn banner_frame(
		ctx: &egui::Context,
		view: &mut TimelineView,
		state: &mut State,
		events: Vec<egui::Event>,
		shift_widget_order: bool,
	) -> Vec<(String, egui::Rect)> {
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
		let output = ctx.run_ui(
			egui::RawInput {
				focused: true,
				events,
				screen_rect: Some(egui::Rect::from_min_size(
					egui::Pos2::ZERO,
					egui::vec2(900.0, 600.0),
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
					(&mut crate::avatars::Avatars::default(), &mut None),
					None,
				);
			},
		);
		assert!(output.platform_output.commands.is_empty());
		let mut labels = vec![];
		for shape in &output.shapes {
			collect(&shape.shape, &mut labels);
		}
		output.drop_without_applying_deltas();
		labels
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
					assert!(view.target_browsing && view.mark_read.is_none());
					assert!(view.anchor.is_some_and(|(_, inset)| inset <= 3.0));
					continue;
				}
				for forbidden in [
					"Unread messages",
					"More messages",
					"Jump to unread",
					"New messages below",
					"Jump to present",
				] {
					assert!(
						!labels.iter().any(|(text, _)| text == forbidden),
						"{count} messages unexpectedly showed {forbidden}"
					);
				}
				assert!(!view.target_browsing && view.following);
			}
			// Stale latest metadata (a deleted message) is acknowledged like the official
			// client does, otherwise the channel would stay unread forever.
			assert_eq!(view.mark_read, (!tall).then_some(Id(latest)));
			if tall {
				// Scrolling to the live edge of tall unread content resolves both banners,
				// even when the service latest ID names a deleted message.
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
						},
					],
					false,
				);
				assert!(view.following && !view.target_browsing);
				assert_eq!(view.mark_read.take(), Some(Id(21)));
				for forbidden in ["Next messages", "New messages below", "Jump to present"] {
					assert!(
						!labels.iter().any(|(text, _)| text == forbidden),
						"tall stale channel still showed {forbidden}"
					);
				}
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
		for (distance, expected) in [(1_200.0, false), (2_400.0, true)] {
			let mut labels = vec![];
			for _ in 0..5 {
				let offset =
					view.rows.iter().map(|(_, height)| height).sum::<f32>() - 600.0 - distance;
				let (index, _, top) = visible_range(&view.rows, offset, offset);
				view.following = false;
				view.jump = false;
				view.anchor = Some((view.rows[index].0, offset - top));
				view.revision = u64::MAX;
				labels = banner_frame(&ctx, &mut view, &mut state, vec![], false);
			}
			assert_eq!(
				labels.iter().any(|(text, _)| text == "Jump to present"),
				expected
			);
		}
		let labels = banner_frame(&ctx, &mut view, &mut state, vec![], false);
		let pos = labels
			.iter()
			.find(|(text, _)| text == "Jump to present")
			.unwrap()
			.1
			.center();
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
			view.following && view.jump,
			"Jump to present lost its click when sibling widget IDs changed"
		);
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
						(&mut crate::avatars::Avatars::default(), &mut None),
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
		view.follow_latest();
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
			id: Id(id),
			channel: Id(20),
			author: model::User {
				id: Id(2),
				name: "Robin".into(),
				avatar: None,
				webhook: false,
				kind: Default::default(),
				discriminator: 0,
			},
			content: "Synthetic text with enough words to wrap in a narrow viewport.".into(),
			edited: false,
			edited_at: None,
			revision: 0,
			nonce: None,
			reply_to: None,
			kind: 0,
			reply_deleted: false,
			forwarded: false,
			unsupported: false,
			extra_content: Default::default(),
			embeds: vec![],
			attachments: vec![],
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
							(&mut avatars, &mut None),
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
							(&message, &[], &mut None),
							(own, true, true, can_delete),
							(None, &mut reply),
							(&mut editing, &mut edit_started),
							&mut deleting,
							(false, false, &mut None),
						)
					},
				);
				assert!(output.platform_output.commands.is_empty());
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
							(&mut images, &mut None),
							None,
						)
					},
				);
				assert!(output.platform_output.commands.is_empty());
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
			for events in click("Reveal spoiler", &labels) {
				render(&mut view, &mut state, events);
			}
			assert!(!view.revealed.is_empty());
			state.selected = Some(Id(30));
			render(&mut view, &mut state, vec![]);
			assert!(view.revealed.is_empty());
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
							(&mut avatars, &mut None),
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
		assert_ne!(
			row_key(&next, Some(&first), None),
			row_key(&next, None, None)
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
					(&mut avatars, &mut None),
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
				(&mut avatars, &mut None),
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
							(&mut images, &mut None),
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
							(&mut avatars, &mut None),
							None,
						)
					},
				);
				assert!(output.platform_output.commands.is_empty());
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
				view.opening,
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
						(&mut avatars, &mut None),
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
				assert_eq!(view.opening, discord_url(&channel, Some(message.id)));
				view.opening = None;
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
								(&mut avatars, &mut None),
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
			assert_eq!(view.toolbar.unwrap().1.width(), 120.0);
			assert_eq!(view.heights, heights);
			let point = view.toolbar.unwrap().1.left_top() + egui::vec2(44.0, 14.0);
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
			assert_eq!(state.reply.take(), Some(Id(60_000 << 22)));
			// Both entry points share the same menu, including on selectable text,
			// row whitespace, and Shift+right-click (which must not quick-delete).
			let menu_labels = [
				"Copy message",
				"Reply",
				"Mark read through here",
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
				assert_eq!(state.reply.take(), Some(Id(60_000 << 22)));
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
				// Reply is the second 28px icon in the retained toolbar.
				if focused.is_some_and(|r| {
					view.toolbar.is_some_and(|(_, toolbar)| {
						toolbar.contains_rect(r.rect)
							&& r.rect.width() < 40.0
							&& (r.rect.left() - (toolbar.left() + 30.0)).abs() < 3.0
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
							(&mut avatars, &mut None),
							None,
						)
					},
				);
				assert!(output.platform_output.commands.is_empty());
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
			let labels = frame(&mut view, &mut state, vec![]);
			let pos = labels
				.iter()
				.find(|(text, _)| text.contains("Jump to present"))
				.unwrap()
				.1
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

	#[test]
	fn initial_unread_gap_stays_unacknowledged_and_keyboard_jump_is_explicit() {
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
							(&mut avatars, &mut None),
							None,
						);
						assert!(ui.min_rect().right() <= ui.max_rect().right() + 1.0);
					},
				);
				assert!(output.platform_output.commands.is_empty());
				let unread_focused = output.platform_output.events.iter().any(|event| {
					matches!(event, egui::output::OutputEvent::FocusGained(info)
						if info.typ == egui::WidgetType::Button
							&& info.enabled
							&& info.label.as_deref() == Some("Jump to unread"))
				});
				output.drop_without_applying_deltas();
				unread_focused
			};
			for _ in 0..3 {
				frame(&mut view, &mut state, vec![]);
			}
			if marker == Some(Id(19)) {
				assert_eq!(
					view.mark_read.take(),
					Some(Id(20)),
					"Loaded read boundary preserves ordinary auto-read"
				);
				continue;
			}
			assert!(view.target_browsing && !view.following);
			assert!(view.mark_read.is_none());
			let key = |key| egui::Event::Key {
				key,
				physical_key: None,
				pressed: true,
				repeat: false,
				modifiers: egui::Modifiers::NONE,
			};
			let mut unread_focused = false;
			// The overlay follows the keyboard-accessible message rows in widget order.
			for _ in 0..32 {
				unread_focused = frame(&mut view, &mut state, vec![key(egui::Key::Tab)]);
				assert!(view.mark_read.is_none() && !view.unread_jump);
				if unread_focused {
					break;
				}
			}
			assert!(
				unread_focused,
				"Keyboard navigation must reach Jump to unread"
			);
			frame(&mut view, &mut state, vec![key(egui::Key::Enter)]);
			assert!(view.unread_jump);
			assert!(view.mark_read.is_none());
			assert_eq!(state.read_marker(Id(20)), Some(marker));

			// Scrolling to the live edge also resumes reading without clicking a banner.
			view.unread_jump = false;
			frame(
				&mut view,
				&mut state,
				vec![
					egui::Event::PointerMoved(egui::pos2(width / 2.0, 300.0)),
					egui::Event::MouseWheel {
						unit: egui::MouseWheelUnit::Point,
						delta: egui::vec2(0.0, -600.0),
						modifiers: egui::Modifiers::NONE,
						phase: egui::TouchPhase::Move,
					},
				],
			);
			assert!(view.following && !view.target_browsing);
			assert_eq!(view.mark_read.take(), Some(Id(20)));
		}
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
						(&mut avatars, &mut None),
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
						(&mut avatars, &mut None),
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
			let mut view = TimelineView::default();
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
							},
						],
						..Default::default()
					},
					|ui| {
						view.show(
							ui,
							&mut state,
							&mut None,
							&mut None,
							(&mut avatars, &mut None),
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
							(&mut avatars, &mut None),
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
				"Only visible rows and overscan are measured"
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
			state.timeline.delete(anchor.0).unwrap();
			state.revision += 1;
			for _ in 0..4 {
				render(&mut view, &mut state);
			}
			assert_eq!(
				view.anchor.unwrap().0,
				Id(anchor.0.0 + 1),
				"Deleting the anchored row keeps the next surviving message at the top"
			);
			assert!(!view.heights.contains_key(&anchor.0));
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
								(&mut avatars, &mut None),
								None,
							);
							assert!(ui.min_rect().right() <= ui.max_rect().right() + 1.0);
						},
					);
					assert!(output.platform_output.commands.is_empty());
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
					if deleted_only { vec![] } else { expected }
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
				if deleted_only {
					assert!(state.timeline.is_empty());
					assert!(!labels.iter().any(|text| text == "Message deleted"));
					assert!(!labels.iter().any(|text| text.contains("Resident alpha")
						|| text == "Loading messages?"
						|| text.contains("No messages yet")));
				} else {
					assert!(labels.iter().any(|text| text.contains("Resident alpha")));
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
		assert_eq!(anchor_offset(&neighbors, Id(3), 200.0), 140.0);
		assert_eq!(anchor_offset(&[], Id(2), 25.0), 0.0);
	}

	#[test]
	fn deleted_only_timeline_discards_content_and_has_no_message_actions() {
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
							(&mut avatars, &mut None),
							None,
						);
						assert!(ui.min_rect().right() <= ui.max_rect().right() + 1.0);
					},
				);
				assert!(output.platform_output.commands.is_empty());
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
			state.timeline.delete(message.id).unwrap();
			state.revision += 1;
			for _ in 0..3 {
				render(&mut view, &mut state);
			}
			let labels = render(&mut view, &mut state);
			assert!(state.timeline.is_empty());
			assert_eq!(state.timeline.row_count(), 1);
			assert!(view.rows.is_empty());
			assert!(!labels.iter().any(|text| text.contains("January 1, 2015")));
			for text in [
				"Deleted synthetic author",
				"Deleted synthetic body",
				"Reveal spoiler",
				"Reply",
				"Open in Discord",
				"Message deleted",
			] {
				assert!(
					!labels.iter().any(|label| label.contains(text)),
					"Deleted row exposed {text}"
				);
			}
			assert!(view.revealed.is_empty() && view.viewing.is_none() && view.toolbar.is_none());
			assert!(view.reaction.is_none());
			// Only the channel's own latest ID may be acknowledged, never the deleted row.
			let selected = state.channel(state.selected.unwrap()).unwrap();
			assert_ne!(selected.last_message, Some(message.id));
			assert_eq!(view.mark_read, selected.last_message);
		}
	}
	#[test]
	fn channel_rename_invalidates_offscreen_reference_heights() {
		let message = Message {
			id: Id(1),
			channel: Id(2),
			author: model::User {
				id: Id(3),
				name: "Synthetic".into(),
				avatar: None,
				webhook: false,
				kind: Default::default(),
				discriminator: 0,
			},
			content: "<#4> ".repeat(12),
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
			forwarded: false,
			unsupported: false,
			extra_content: Default::default(),
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
						|ui| view.show(ui, state, &mut None, &mut None, (images, &mut None), None),
					)
					.drop_without_applying_deltas();
			};
		for _ in 0..3 {
			render(&mut view, &mut state, &mut images);
		}
		let short_height = view.heights[&Id(1)].1;
		view.following = false;
		view.anchor = Some((Id(2), 400.0));
		state.revision += 1;
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
		state.revision += 1;
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
	fn navigation_preserves_active_download_controls() {
		let mut view = TimelineView::default();
		view.download.active = true;
		view.download.status = "Downloading: 1 / 2 KiB".into();
		view.download.cancel_requested = true;
		let mut state = State::default();
		let context = egui::Context::default();
		for channel in [Some(Id(2)), Some(Id(3)), None] {
			state.selected = channel;
			context
				.run_ui(Default::default(), |ui| {
					view.show(
						ui,
						&mut state,
						&mut None,
						&mut None,
						(&mut crate::avatars::Avatars::default(), &mut None),
						None,
					);
				})
				.drop_without_applying_deltas();
			assert!(view.download.active && view.download.cancel_requested);
			assert_eq!(view.download.status, "Downloading: 1 / 2 KiB");
		}
	}
	#[test]
	fn same_id_revision_reset_does_not_reuse_reveal_or_height() {
		let mut message = Message {
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
			},
			content: "||old revealed content||".into(),
			edited: false,
			edited_at: None,
			revision: 0,
			nonce: None,
			reply_to: None,
			kind: 0,
			reply_deleted: false,
			forwarded: false,
			unsupported: false,
			extra_content: Default::default(),
			embeds: vec![],
			attachments: vec![],
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
				(&mut crate::avatars::Avatars::default(), &mut None),
				None,
			);
		});
		output.drop_without_applying_deltas();
		assert!(view.revealed.is_empty());
		assert_eq!(
			view.heights[&Id(1)].0,
			row_key(state.timeline.get(Id(1)).unwrap(), None, None)
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
			},
			content: "Ordinary text".into(),
			edited: false,
			edited_at: None,
			revision: 0,
			nonce: None,
			reply_to: None,
			kind: 0,
			reply_deleted: false,
			forwarded: false,
			unsupported: false,
			extra_content: Default::default(),
			attachments: vec![],
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
				let _ = super::super::embeds::show(
					ui,
					&message,
					&mut FormatCache::default(),
					&mut crate::avatars::Avatars::default(),
					&mut None,
					&mut crate::attachments::DownloadUi::default(),
					&mut None,
					&State {
						demo: true,
						..Default::default()
					},
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
					|ui| view.show(ui, state, &mut None, &mut None, (images, &mut None), None),
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
				.all(|key| !key.starts_with("embed:")),
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
		assert_eq!(requests.len(), 2);
		assert!(requests.iter().any(|key| key.starts_with("embed:")));
		assert!(requests.iter().any(|key| key.starts_with("large:")));
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
}
