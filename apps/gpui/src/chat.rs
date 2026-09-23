//! Conversation header, grouped timeline and composer, following the egui timeline's metrics.
use crate::sidebar::avatar;
use crate::theme::{FONT, Icon, color, icon, palette, tint};
use crate::{Serein, channel_label, tooltip};
use client_core::State;
use gpui::{prelude::*, *};
use model::{Id, Message};
use std::ops::Range;

/// Consecutive messages from one author within this window share a header.
const GROUP_SECONDS: i64 = 300;
const BODY: f32 = 15.;
#[cfg(target_os = "macos")]
const MONO: &str = "Menlo";
#[cfg(target_os = "windows")]
const MONO: &str = "Consolas";
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
const MONO: &str = "DejaVu Sans Mono";

/// Local time of a snowflake's creation.
fn created(id: Id) -> Option<time::OffsetDateTime> {
	let millis = (id.0 >> 22) as i128 + 1_420_070_400_000;
	time::OffsetDateTime::from_unix_timestamp_nanos(millis * 1_000_000)
		.ok()
		.map(ui::local_datetime)
}
fn clock(id: Id) -> String {
	created(id).map_or_else(String::new, |at| {
		format!("{:02}:{:02}", at.hour(), at.minute())
	})
}
/// "Sep 10, 2026 14:03" for search results.
pub(crate) fn short_date(id: Id) -> String {
	created(id).map_or_else(String::new, |at| {
		let month = format!("{}", at.month());
		format!(
			"{} {}, {} {:02}:{:02}",
			&month[..3.min(month.len())],
			at.day(),
			at.year(),
			at.hour(),
			at.minute()
		)
	})
}
/// "September 10, 2026" for when a snowflake was created (account age on profiles).
pub(crate) fn day_label(id: Id) -> String {
	created(id).map_or_else(String::new, date_label)
}
fn date_label(at: time::OffsetDateTime) -> String {
	format!("{} {}, {}", at.month(), at.day(), at.year())
}
fn format_size(bytes: u64) -> String {
	match bytes {
		0..1024 => format!("{bytes} bytes"),
		1024..1_048_576 => format!("{:.1} KB", bytes as f64 / 1024.),
		_ => format!("{:.1} MB", bytes as f64 / 1_048_576.),
	}
}
/// The same grouping rule as the egui timeline.
fn continues(previous: &Message, message: &Message, boundary: Option<Id>) -> bool {
	let (Some(a), Some(b)) = (created(previous.id), created(message.id)) else {
		return false;
	};
	previous.author.id == message.author.id
		&& previous.author.account_label() == message.author.account_label()
		&& message.reply_to.is_none()
		&& !previous.unsupported
		&& !message.unsupported
		&& !previous.extra_content.any()
		&& !message.extra_content.any()
		&& !previous.is_system()
		&& !message.is_system()
		&& boundary != Some(message.id)
		&& a.date() == b.date()
		&& (b - a).whole_seconds() < GROUP_SECONDS
}

#[derive(Clone)]
enum Click {
	Link(String),
	Channel(Id),
	Spoiler,
}

/// One run of text with the same block kind, before it becomes a `StyledText`.
#[derive(Default)]
struct Paragraph {
	text: String,
	runs: Vec<TextRun>,
	clicks: Vec<(Range<usize>, Click)>,
	quote: bool,
	heading: u8,
	small: bool,
}

struct Markdown<'a> {
	state: &'a State,
	message: &'a Message,
	revealed: bool,
	id: Id,
	part: u16,
	output: Vec<AnyElement>,
	current: Paragraph,
	blocks_done: usize,
}
impl Markdown<'_> {
	fn flush(&mut self, view: &Entity<Serein>) {
		let mut paragraph = std::mem::take(&mut self.current);
		let trimmed = paragraph.text.trim_end_matches(['\n', '\r']).len();
		if trimmed == 0 {
			return;
		}
		// Keep runs covering exactly the trimmed text.
		let mut remaining = trimmed;
		paragraph.runs.retain_mut(|run| {
			if remaining == 0 {
				return false;
			}
			run.len = run.len.min(remaining);
			remaining -= run.len;
			true
		});
		paragraph.text.truncate(trimmed);
		let p = palette();
		let index = self.output.len();
		let styled = StyledText::new(paragraph.text).with_runs(paragraph.runs);
		let ranges = paragraph
			.clicks
			.iter()
			.map(|(range, _)| range.clone())
			.collect::<Vec<_>>();
		let clicks = paragraph
			.clicks
			.into_iter()
			.map(|(_, click)| click)
			.collect::<Vec<_>>();
		let message = self.id;
		let view = view.downgrade();
		let text = InteractiveText::new(
			ElementId::NamedInteger(format!("body-{}-{index}", self.part).into(), message.0),
			styled,
		)
		.on_click(ranges, move |ix, window, cx| {
			let Some(click) = clicks.get(ix).cloned() else {
				return;
			};
			match click {
				Click::Link(url) => confirm_open(url, window, cx),
				Click::Channel(channel) => {
					let _ =
						view.update(cx, |this, cx| {
							if this.state.channel(channel).is_some_and(|c| {
								crate::text_channel(c) && this.state.can_view(c.id)
							}) {
								this.select(channel, cx);
							}
						});
				}
				Click::Spoiler => {
					let _ = view.update(cx, |this, cx| {
						this.revealed.insert(message);
						cx.notify();
					});
				}
			}
		});
		let size = match (paragraph.heading, paragraph.small) {
			(1, _) => BODY * 1.5,
			(2, _) => BODY * 1.25,
			(_, true) => BODY * 0.8,
			_ => BODY,
		};
		let element = div()
			.w_full()
			.text_size(px(size))
			.line_height(px((size * 1.375).round()))
			.when(paragraph.heading > 0, |d| d.mt_1())
			.child(text);
		self.output.push(if paragraph.quote {
			div()
				.flex()
				.gap(px(8.))
				.child(
					div()
						.w(px(4.))
						.flex_none()
						.rounded(px(2.))
						.bg(color(p.selected)),
				)
				.child(div().flex_1().min_w_0().child(element))
				.into_any_element()
		} else {
			element.into_any_element()
		});
	}

	fn push(&mut self, span: ui::MarkdownSpan<'_>, view: &Entity<Serein>) {
		let p = palette();
		if span.quote != self.current.quote
			|| span.heading != self.current.heading
			|| span.small != self.current.small
		{
			self.flush(view);
			self.current.quote = span.quote;
			self.current.heading = span.heading;
			self.current.small = span.small;
		}
		let mut click = None;
		let mut background = None;
		let mut strong = span.strong || span.heading > 0;
		let mut foreground = if span.small { p.muted } else { p.text };
		let text: String = if let Some(user) = span.mention {
			strong = true;
			foreground = p.mention_text;
			background = Some(tint(p.accent, 0.3));
			let name = self
				.message
				.mentions
				.iter()
				.find(|u| u.id == user)
				.map_or_else(
					|| user.to_string(),
					|u| self.state.user_display_name(u).to_owned(),
				);
			format!("@{name}")
		} else if let Some(role) = span.role {
			strong = true;
			foreground = p.mention_text;
			background = Some(tint(p.accent, 0.3));
			let name = self
				.state
				.channel(self.message.channel)
				.and_then(|c| c.guild)
				.and_then(|guild| self.state.guild_roles(guild))
				.and_then(|roles| roles.iter().find(|r| r.id == role))
				.map_or_else(|| format!("unknown-role ({role})"), |r| r.name.clone());
			format!("@{name}")
		} else if let Some(channel) = span.channel {
			strong = true;
			foreground = p.link;
			click = Some(Click::Channel(channel));
			match self.state.channel(channel) {
				Some(channel) => format!("#{}", channel_label(channel)),
				None => "#unknown-channel".into(),
			}
		} else if let Some((seconds, style)) = span.timestamp {
			background = Some(tint(p.raised, 1.));
			ui::discord_timestamp(seconds, style).unwrap_or_else(|| span.text.to_owned())
		} else {
			if span.mass_mention {
				strong = true;
				foreground = p.mention_text;
				background = Some(tint(p.accent, 0.3));
			}
			// Text runs cannot hold images: inline custom emoji read as `:name:`.
			custom_emoji_names(span.text)
		};
		if let Some(url) = span.link
			&& click.is_none()
		{
			foreground = p.link;
			click = Some(Click::Link(url.to_owned()));
		}
		if span.code {
			background = Some(tint(p.raised, 1.));
		}
		let mut hsla: Hsla = color(foreground).into();
		if span.spoiler && !self.revealed {
			background = Some(tint(p.selected, 1.));
			hsla = color(p.selected).into();
			click = Some(Click::Spoiler);
		}
		let start = self.current.text.len();
		self.current.text.push_str(&text);
		let len = text.len();
		if len == 0 {
			return;
		}
		if let Some(click) = click {
			self.current.clicks.push((start..start + len, click));
		}
		let mut font = font(if span.code { MONO } else { FONT });
		font.weight = if strong {
			FontWeight::SEMIBOLD
		} else {
			FontWeight::NORMAL
		};
		if span.italic {
			font.style = FontStyle::Italic;
		}
		self.current.runs.push(TextRun {
			len,
			font,
			color: hsla,
			background_color: background.map(Into::into),
			underline: (span.underline || span.link.is_some()).then(|| UnderlineStyle {
				thickness: px(1.),
				color: Some(hsla),
				wavy: false,
			}),
			strikethrough: span.strike.then(|| StrikethroughStyle {
				thickness: px(1.),
				color: Some(hsla),
			}),
		});
	}
}

/// Name of a custom emoji token `<:name:id>` / `<a:name:id>`.
fn custom_name(token: &str) -> &str {
	token
		.trim_start_matches("<a:")
		.trim_start_matches("<:")
		.split(':')
		.next()
		.unwrap_or_default()
}

fn custom_emoji_names(text: &str) -> String {
	let mut out = String::with_capacity(text.len());
	let mut offset = 0;
	while offset < text.len() {
		if let Some((_, len)) = ui::emoji::custom_prefix(&text[offset..]) {
			out.push(':');
			out.push_str(custom_name(&text[offset..offset + len]));
			out.push(':');
			offset += len;
		} else {
			let next = text[offset..].chars().next().map_or(1, char::len_utf8);
			out.push_str(&text[offset..offset + next]);
			offset += next;
		}
	}
	out
}

/// A custom emoji image once cached, else its `:name:` in muted text.
pub(crate) fn custom_emoji(id: Id, name: &str, size: f32) -> AnyElement {
	match crate::images::get(&format!("emoji-{id}")) {
		Some(image) => img(image)
			.size(px(size))
			.flex_none()
			.object_fit(ObjectFit::Contain)
			.into_any_element(),
		None => div()
			.text_size(px((size * 0.6).max(13.)))
			.text_color(color(palette().muted))
			.child(format!(":{name}:"))
			.into_any_element(),
	}
}

/// Emoji-only messages render large, custom emoji as images, like the main app.
fn jumbo(spans: impl Iterator<Item = String>) -> AnyElement {
	use unicode_segmentation::UnicodeSegmentation;
	let mut items = Vec::new();
	for text in spans {
		let mut offset = 0;
		while offset < text.len() {
			if let Some((id, len)) = ui::emoji::custom_prefix(&text[offset..]) {
				items.push(custom_emoji(
					id,
					custom_name(&text[offset..offset + len]),
					48.,
				));
				offset += len;
				continue;
			}
			let Some(cluster) = text[offset..].graphemes(true).next() else {
				break;
			};
			if !cluster.trim().is_empty() {
				items.push(
					div()
						.text_size(px(44.))
						.child(cluster.to_owned())
						.into_any_element(),
				);
			}
			offset += cluster.len();
		}
	}
	div()
		.flex()
		.flex_wrap()
		.items_center()
		.gap_1()
		.children(items)
		.into_any_element()
}

/// One-line text of a message with mentions resolved, for reply previews.
fn preview_text(format: &mut ui::FormatCache, state: &State, message: &Message) -> String {
	let source = message.display_text();
	let mut text = String::new();
	for span in format.get(message.id, &source).spans() {
		if text.chars().count() >= 160 {
			break;
		}
		if let Some(user) = span.mention {
			let name = message.mentions.iter().find(|u| u.id == user);
			text.push('@');
			text.push_str(name.map_or("unknown-user", |u| state.user_display_name(u)));
		} else if let Some(channel) = span.channel {
			text.push('#');
			text.push_str(
				&state
					.channel(channel)
					.map_or_else(|| "unknown-channel".into(), channel_label),
			);
		} else if span.role.is_some() {
			text.push_str("@role");
		} else {
			text.extend(
				span.text
					.chars()
					.map(|c| if matches!(c, '\n' | '\r') { ' ' } else { c }),
			);
		}
	}
	text.chars().take(160).collect()
}

/// Highlighted fenced block with the main app's syntax colours and a copy button.
fn code_block(
	tag: &str,
	code: &str,
	tokens: &[(u32, u32, ui::CodeToken)],
	message: Id,
	index: usize,
	cx: &mut Context<Serein>,
) -> AnyElement {
	let p = palette();
	// Light palettes have bright chat surfaces; pick the matching syntax set.
	let [r, g, b, _] = p.chat.to_array();
	let dark = u32::from(r) * 299 + u32::from(g) * 587 + u32::from(b) * 114 < 128_000;
	let colors = ui::design::code_colors_for(p, dark);
	let mut runs = Vec::with_capacity(tokens.len().max(1));
	let mut covered = 0;
	for &(start, end, token) in tokens {
		let (start, end) = (start as usize, (end as usize).min(code.len()));
		if start < covered
			|| start >= end
			|| !code.is_char_boundary(start)
			|| !code.is_char_boundary(end)
		{
			continue;
		}
		if start > covered {
			runs.push(code_run(covered, start, p.text, false));
		}
		runs.push(code_run(
			start,
			end,
			colors.color(token, p.text),
			token == ui::CodeToken::Comment,
		));
		covered = end;
	}
	if covered < code.len() {
		runs.push(code_run(covered, code.len(), p.text, false));
	}
	let copy = code.to_owned();
	div()
		.my_1()
		.max_w(px(720.))
		.px(px(10.))
		.py(px(8.))
		.rounded(px(6.))
		.bg(color(p.raised))
		.border_1()
		.border_color(color(p.border))
		.flex()
		.flex_col()
		.gap_1()
		.child(
			div()
				.flex()
				.items_center()
				.justify_between()
				.pb_1()
				.border_b_1()
				.border_color(color(p.border))
				.child(
					div()
						.text_size(px(12.))
						.font_weight(FontWeight::SEMIBOLD)
						.text_color(color(p.muted))
						.child(if tag.is_empty() {
							"code".to_owned()
						} else {
							tag.to_owned()
						}),
				)
				.child(
					div()
						.id(ElementId::NamedInteger(
							format!("copy-code-{index}").into(),
							message.0,
						))
						.px_2()
						.rounded(px(4.))
						.cursor_pointer()
						.text_size(px(12.))
						.text_color(color(p.muted))
						.hover(|d| d.bg(color(p.hover)).text_color(color(p.text_strong)))
						.on_click(cx.listener(move |this, _, _, cx| {
							cx.write_to_clipboard(ClipboardItem::new_string(copy.clone()));
							this.notify_user("Code copied");
							cx.notify();
						}))
						.child("Copy"),
				),
		)
		.child(
			div()
				.text_size(px(13.5))
				.line_height(px(19.))
				.child(StyledText::new(code.to_owned()).with_runs(runs)),
		)
		.into_any_element()
}

fn code_run(start: usize, end: usize, tone: egui::Color32, italic: bool) -> TextRun {
	let mut font = font(MONO);
	if italic {
		font.style = FontStyle::Italic;
	}
	TextRun {
		len: end - start,
		font,
		color: color(tone).into(),
		background_color: None,
		underline: None,
		strikethrough: None,
	}
}

/// A cached embed image or thumbnail, sized from its metadata; nothing until it has loaded
/// or when images are off (demo).
fn embed_media(media: &model::EmbedMedia, bounds: (u32, u32)) -> Option<AnyElement> {
	if !crate::images::enabled() {
		return None;
	}
	let key = crate::images::media_key(media)?;
	let image = crate::images::get(&key);
	let (width, height) = crate::images::fit(media.width, media.height, bounds);
	Some(
		div()
			.mt_1()
			.w(px(width as f32))
			.h(px(height as f32))
			.rounded(px(4.))
			.overflow_hidden()
			.bg(color(palette().chat))
			.children(image.map(|image| img(image).size_full().object_fit(ObjectFit::Contain)))
			.into_any_element(),
	)
}

pub(crate) fn confirm_open_link(url: String, window: &mut Window, cx: &mut App) {
	confirm_open(url, window, cx)
}

fn confirm_open(url: String, window: &mut Window, cx: &mut App) {
	let answer = window.prompt(
		PromptLevel::Info,
		"Open this link in your browser?",
		Some(&url),
		&["Open", "Cancel"],
		cx,
	);
	cx.spawn(async move |cx| {
		if answer.await == Ok(0) {
			cx.update(|cx| cx.open_url(&url));
		}
	})
	.detach();
}

impl Serein {
	fn body(&mut self, message: &Message, cx: &mut Context<Self>) -> Vec<AnyElement> {
		let source = message.display_text().into_owned();
		self.markdown(message, 0, &source, cx)
	}

	/// Markdown for one part of a message: 0 is the body, later parts are component texts.
	pub(crate) fn markdown(
		&mut self,
		message: &Message,
		part: u16,
		source: &str,
		cx: &mut Context<Self>,
	) -> Vec<AnyElement> {
		if source.trim().is_empty() {
			return Vec::new();
		}
		let view = cx.entity();
		let formatted = self.format.get_part(message.id, part, source);
		if formatted.jumbo() {
			return vec![jumbo(formatted.spans().map(|span| span.text.to_owned()))];
		}
		let mut markdown = Markdown {
			state: &self.state,
			message,
			revealed: self.revealed.contains(&message.id),
			id: message.id,
			part,
			output: Vec::new(),
			current: Paragraph::default(),
			blocks_done: 0,
		};
		for span in formatted.spans() {
			if let Some(block) = span.block {
				if block >= markdown.blocks_done {
					markdown.flush(&view);
					markdown.blocks_done = block + 1;
					if let Some((tag, code)) = formatted.code_block(block) {
						let tokens = formatted.code_tokens(block);
						let index = usize::from(part) * 64 + block;
						markdown
							.output
							.push(code_block(tag, code, tokens, message.id, index, cx));
					}
				}
				continue;
			}
			markdown.push(span, &view);
		}
		markdown.flush(&view);
		markdown.output
	}

	fn divider(label: String, danger: bool) -> impl IntoElement {
		let p = palette();
		let line = color(if danger { p.danger } else { p.border });
		div()
			.mx_4()
			.mt(px(16.))
			.mb(px(4.))
			.h(px(20.))
			.flex()
			.items_center()
			.gap(px(12.))
			.child(div().flex_1().h(px(1.)).bg(line))
			.child(
				div()
					.text_size(px(12.))
					.font_weight(FontWeight::SEMIBOLD)
					.text_color(color(if danger { p.danger } else { p.muted }))
					.child(label),
			)
			.child(div().flex_1().h(px(1.)).bg(line))
	}

	fn reply_line(
		&mut self,
		message: &Message,
		cx: &mut Context<Self>,
	) -> Option<impl IntoElement> {
		let p = palette();
		let reply = message.reply_to?;
		let openable = self.state.can_open_reply_target(reply);
		let original = self.state.timeline.get(reply);
		let connector = div().w(px(50.)).h(px(18.)).flex_none().relative().child(
			div()
				.absolute()
				.left(px(19.))
				.top(px(8.))
				.w(px(31.))
				.h(px(12.))
				.border_l_2()
				.border_t_2()
				.rounded_tl(px(5.))
				.border_color(tint(p.muted, 0.5)),
		);
		let content = if message.reply_deleted || self.state.timeline.is_deleted(reply) {
			div()
				.text_size(px(13.))
				.italic()
				.text_color(color(p.muted))
				.child("Message deleted")
				.into_any_element()
		} else if let Some(original) = original {
			let name = self.state.message_author_name(original).to_owned();
			let preview = preview_text(&mut self.format, &self.state, original);
			div()
				.flex()
				.items_center()
				.gap(px(6.))
				.min_w_0()
				.child(avatar(&name, 16., Some(&original.author)))
				.child(
					div()
						.flex_none()
						.text_size(px(13.))
						.font_weight(FontWeight::SEMIBOLD)
						.text_color(color(p.muted))
						.child(format!("@{name}")),
				)
				.child(
					div()
						.min_w_0()
						.overflow_hidden()
						.whitespace_nowrap()
						.text_ellipsis()
						.text_size(px(13.))
						.text_color(color(p.muted))
						.child(if preview.is_empty() {
							"Click to see attachment".to_owned()
						} else {
							preview
						}),
				)
				.into_any_element()
		} else {
			div()
				.text_size(px(13.))
				.text_color(color(p.muted))
				.child("Earlier message")
				.into_any_element()
		};
		Some(
			div()
				.id(("reply-line", message.id.0))
				.h(px(18.))
				.mb(px(4.))
				.flex()
				.items_center()
				.gap(px(6.))
				.overflow_hidden()
				.when(openable, |d| {
					d.cursor_pointer()
						.hover(|d| d.opacity(0.8))
						.on_click(cx.listener(move |this, _, _, cx| this.open_reply(reply, cx)))
				})
				.child(connector)
				.child(content),
		)
	}

	/// Loads or scrolls to the replied-to message without sending anything.
	fn open_reply(&mut self, target: Id, cx: &mut Context<Self>) {
		let command = self.state.open_reply_target(target);
		self.dispatch(command);
		self.sync_rows();
		if let Some(index) = self.rows.iter().position(|id| *id == target) {
			self.messages.scroll_to_reveal_item(index);
		}
		cx.notify();
	}

	fn attachment(
		&self,
		attachment: &model::Attachment,
		ix: usize,
		message: Id,
	) -> impl IntoElement {
		let p = palette();
		let name = attachment.filename.to_ascii_lowercase();
		let kind = attachment.content_type.as_deref().unwrap_or("");
		let (glyph, tone) = if kind.starts_with("image/") || attachment.is_video() {
			(Icon::FileImage, p.accent)
		} else if name.ends_with(".pdf") {
			(Icon::File, p.danger)
		} else if [".zip", ".7z", ".rar", ".tar", ".gz"]
			.iter()
			.any(|e| name.ends_with(e))
		{
			(Icon::File, p.warning)
		} else if kind.starts_with("text/") || name.ends_with(".txt") || name.ends_with(".md") {
			(Icon::FileText, p.link)
		} else {
			(Icon::File, p.muted)
		};
		let url = attachment.media.url.clone();
		// Inline preview, sized from the attachment metadata so its arrival never moves rows.
		if kind.starts_with("image/")
			&& kind != "image/svg+xml"
			&& !attachment.spoiler
			&& crate::images::enabled()
			&& let Some(key) = crate::images::media_key(&attachment.media)
		{
			let media = &attachment.media;
			let (width, height) =
				crate::images::fit(media.width, media.height, crate::images::MEDIA_BOX);
			return div()
				.id(ElementId::NamedInteger(
					format!("attachment-{ix}").into(),
					message.0,
				))
				.mt_1()
				.w(px(width as f32))
				.max_w_full()
				.aspect_ratio(width as f32 / height as f32)
				.rounded(px(8.))
				.overflow_hidden()
				.bg(color(p.raised))
				.flex()
				.items_center()
				.justify_center()
				.tooltip(tooltip(attachment.filename.clone()))
				.when_some(url, |d, url| {
					d.cursor_pointer()
						.on_click(move |_, window, cx| confirm_open(url.clone(), window, cx))
				})
				.child(match crate::images::get(&key) {
					Some(image) => img(image)
						.size_full()
						.object_fit(ObjectFit::Contain)
						.into_any_element(),
					None => icon(Icon::FileImage, px(32.), color(p.muted)).into_any_element(),
				});
		}
		div()
			.id(ElementId::NamedInteger(
				format!("attachment-{ix}").into(),
				message.0,
			))
			.mt_1()
			.max_w(px(432.))
			.px(px(12.))
			.py(px(10.))
			.rounded(px(8.))
			.bg(color(p.raised))
			.border_1()
			.border_color(color(p.border))
			.flex()
			.items_center()
			.gap(px(12.))
			.child(icon(glyph, px(32.), color(tone)))
			.child(
				div()
					.flex_1()
					.min_w_0()
					.flex()
					.flex_col()
					.child(
						div()
							.overflow_hidden()
							.whitespace_nowrap()
							.text_ellipsis()
							.text_size(px(14.))
							.font_weight(FontWeight::MEDIUM)
							.text_color(color(p.link))
							.child(attachment.filename.clone()),
					)
					.child(
						div()
							.text_size(px(12.))
							.text_color(color(p.muted))
							.child(format_size(attachment.size)),
					),
			)
			.children(url.map(|url| {
				div()
					.id("open")
					.size(px(28.))
					.flex_none()
					.rounded(px(6.))
					.flex()
					.items_center()
					.justify_center()
					.cursor_pointer()
					.hover(|d| d.bg(color(p.hover)))
					.tooltip(tooltip("Open in browser"))
					.on_click(move |_, window, cx| confirm_open(url.clone(), window, cx))
					.child(icon(Icon::Download, px(18.), color(p.muted)))
			}))
	}

	fn embed(&self, embed: &model::Embed) -> Option<impl IntoElement> {
		let p = palette();
		if embed.title.is_none()
			&& embed.description.is_none()
			&& embed.fields.is_empty()
			&& embed.author.is_none()
		{
			return None;
		}
		let bar = embed
			.color
			.map_or(color(p.accent), |value| rgb(value & 0xff_ffff));
		let small = |text: String, tone: egui::Color32| {
			div().text_size(px(12.)).text_color(color(tone)).child(text)
		};
		Some(
			div()
				.mt_1()
				.max_w(px(480.))
				.rounded(px(5.))
				.bg(color(p.raised))
				.flex()
				.overflow_hidden()
				.child(
					div()
						.w(px(3.))
						.flex_none()
						.my(px(5.))
						.rounded(px(2.))
						.bg(bar),
				)
				.child(
					div()
						.flex_1()
						.min_w_0()
						.p(px(12.))
						.flex()
						.flex_col()
						.gap_1()
						.children(
							embed
								.provider
								.as_ref()
								.map(|provider| small(provider.name.clone(), p.muted)),
						)
						.children(embed.author.as_ref().map(|author| {
							div()
								.text_size(px(13.))
								.font_weight(FontWeight::SEMIBOLD)
								.text_color(color(p.text_strong))
								.child(author.name.clone())
						}))
						.children(embed.title.as_ref().map(|title| {
							div()
								.font_weight(FontWeight::SEMIBOLD)
								.text_color(color(if embed.url.is_some() {
									p.link
								} else {
									p.text_strong
								}))
								.child(title.clone())
						}))
						.children(embed.description.as_ref().map(|description| {
							div()
								.text_size(px(14.))
								.text_color(color(p.text))
								.child(description.chars().take(600).collect::<String>())
						}))
						.when(!embed.fields.is_empty(), |d| {
							d.child(div().flex().flex_wrap().gap_2().children(
								embed.fields.iter().take(25).map(|field| {
									div()
										.when(field.inline, |d| d.min_w(px(130.)).flex_1())
										.when(!field.inline, |d| d.w_full())
										.flex()
										.flex_col()
										.child(
											div()
												.text_size(px(13.))
												.font_weight(FontWeight::SEMIBOLD)
												.text_color(color(p.text_strong))
												.child(field.name.clone()),
										)
										.child(div().text_size(px(13.)).child(
											field.value.chars().take(300).collect::<String>(),
										))
								}),
							))
						})
						.children(
							embed
								.image
								.as_ref()
								.and_then(|media| embed_media(media, (400, 300))),
						)
						.children(
							embed
								.footer
								.as_ref()
								.map(|footer| small(footer.text.clone(), p.muted)),
						),
				)
				.children(
					embed
						.thumbnail
						.as_ref()
						.filter(|_| embed.image.is_none())
						.and_then(|media| embed_media(media, (84, 84)))
						.map(|thumbnail| div().p(px(12.)).pl_0().flex_none().child(thumbnail)),
				),
		)
	}

	fn reactions(&self, message: &Message, cx: &mut Context<Self>) -> Option<impl IntoElement> {
		let p = palette();
		let reactions = message.reactions.as_ref().filter(|r| !r.is_empty())?;
		let id = message.id;
		Some(
			div()
				.mt_1()
				.flex()
				.flex_wrap()
				.gap_1()
				.children(reactions.iter().enumerate().map(|(ix, reaction)| {
					let emoji = reaction.emoji.clone();
					let label = match (&reaction.emoji.id, &reaction.emoji.name) {
						(Some(_), Some(name)) => format!(":{name}:"),
						(None, Some(name)) => name.clone(),
						_ => "?".into(),
					};
					let me = reaction.me || reaction.me_burst;
					div()
						.id(ElementId::NamedInteger(
							format!("reaction-{ix}").into(),
							id.0,
						))
						.h(px(26.))
						.px(px(6.))
						.rounded(px(6.))
						.flex()
						.items_center()
						.gap_1()
						.cursor_pointer()
						.border_1()
						.when(me, |d| {
							d.bg(tint(p.accent, 0.35)).border_color(color(p.accent))
						})
						.when(!me, |d| {
							d.bg(color(p.raised))
								.border_color(gpui::transparent_black())
								.hover(|d| d.border_color(color(p.muted)))
						})
						.on_click(cx.listener(move |this, _, _, cx| {
							let command = this.state.prepare_reaction(id, emoji.clone());
							if command.is_none() && !this.state.demo {
								this.notify_user("Reactions need a live, fully synced connection.");
							}
							this.dispatch(command);
							cx.notify();
						}))
						.child(match (reaction.emoji.id, &reaction.emoji.name) {
							(Some(id), Some(name)) => custom_emoji(id, name, 18.),
							_ => div().text_size(px(16.)).child(label).into_any_element(),
						})
						.child(
							div()
								.text_size(px(14.))
								.font_weight(FontWeight::MEDIUM)
								.text_color(color(if me { p.text_strong } else { p.text }))
								.child(reaction.count.to_string()),
						)
				})),
		)
	}

	/// Inline editor in place of the body: Enter saves, Escape cancels, as in the main app.
	fn inline_editor(
		&self,
		editor: Entity<crate::input::Input>,
		cx: &mut Context<Self>,
	) -> AnyElement {
		let p = palette();
		let hint = |label: &'static str, action: &'static str| {
			div()
				.flex()
				.gap_1()
				.child(label)
				.child(div().text_color(color(p.link)).child(action))
		};
		div()
			.mt_1()
			.flex()
			.flex_col()
			.gap_1()
			.child(
				div()
					.px(px(10.))
					.py(px(6.))
					.rounded(px(8.))
					.bg(color(p.raised))
					.child(editor),
			)
			.child(
				div()
					.flex()
					.gap_1()
					.text_size(px(12.))
					.text_color(color(p.muted))
					.child(
						div()
							.id("cancel-edit")
							.cursor_pointer()
							.on_click(
								cx.listener(|this, _, window, cx| this.cancel_edit(window, cx)),
							)
							.child(hint("escape to", "cancel")),
					)
					.child("·")
					.child(
						div()
							.id("save-edit")
							.cursor_pointer()
							.on_click(cx.listener(|this, _, _, cx| this.save_edit(cx)))
							.child(hint("enter to", "save")),
					),
			)
			.into_any_element()
	}

	fn toolbar(&self, message: &Message, cx: &mut Context<Self>) -> impl IntoElement {
		let p = palette();
		let id = message.id;
		let text = message.content.clone();
		let channel = message.channel;
		let can_edit = self.state.can_edit(channel, id) && !message.is_system();
		let can_pin = self.state.can_pin(channel, id);
		let pinned = can_pin && self.state.is_pinned(channel, id);
		let can_delete = self.state.can_delete(channel, id);
		let can_react =
			!message.is_system() && (self.state.demo || self.state.can_react(id, None, true));
		div()
			.absolute()
			.top(px(-14.))
			.right(px(16.))
			.h(px(32.))
			.px_1()
			.rounded(px(6.))
			.bg(color(p.raised))
			.border_1()
			.border_color(color(p.border))
			.shadow_md()
			.flex()
			.items_center()
			.when(can_react, |d| {
				d.child(
					self.icon_button(("react", id.0), Icon::Smiley, false, "Add reaction")
						.on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
							let target = crate::emoji::Target::React(id);
							this.open_emoji_picker(target, event.position(), window, cx)
						})),
				)
			})
			.child(
				self.icon_button(("reply", id.0), Icon::Reply, false, "Reply")
					.on_click(cx.listener(move |this, _, window, cx| {
						this.state.reply = Some(client_core::Reply::to(id));
						let focus = this.composer.read(cx).focus_handle(cx);
						window.focus(&focus, cx);
						cx.notify();
					})),
			)
			.when(can_edit, |d| {
				d.child(
					self.icon_button(("edit", id.0), Icon::Pencil, false, "Edit")
						.on_click(
							cx.listener(move |this, _, window, cx| this.start_edit(id, window, cx)),
						),
				)
			})
			.when(can_pin, |d| {
				d.child(
					self.icon_button(
						("pin", id.0),
						Icon::Pin,
						pinned,
						if pinned {
							"Unpin message"
						} else {
							"Pin message"
						},
					)
					.on_click(cx.listener(move |this, _, _, cx| this.toggle_pin(id, cx))),
				)
			})
			.when(!text.is_empty(), |d| {
				d.child(
					self.icon_button(("copy", id.0), Icon::Copy, false, "Copy text")
						.on_click(cx.listener(move |this, _, _, cx| {
							cx.write_to_clipboard(ClipboardItem::new_string(text.clone()));
							this.notify_user("Message text copied");
							cx.notify();
						})),
				)
			})
			.when(can_delete, |d| {
				let p = palette();
				d.child(
					div()
						.id(("delete", id.0))
						.size(px(32.))
						.rounded(px(6.))
						.flex()
						.items_center()
						.justify_center()
						.cursor_pointer()
						.hover(|d| d.bg(tint(p.danger, 0.16)))
						.tooltip(tooltip("Delete message"))
						.on_click(cx.listener(move |this, _, window, cx| {
							this.confirm_delete(id, window, cx)
						}))
						.child(icon(Icon::Trash, px(20.), color(p.danger))),
				)
			})
	}

	pub(crate) fn message(&mut self, ix: usize, cx: &mut Context<Self>) -> AnyElement {
		self.first_rendered.set(self.first_rendered.get().min(ix));
		let p = palette();
		let Some(message) = self
			.rows
			.get(ix)
			.and_then(|id| self.state.timeline.get_display(*id))
			.cloned()
		else {
			return div().into_any_element();
		};
		let previous = ix
			.checked_sub(1)
			.and_then(|i| self.rows.get(i))
			.and_then(|id| self.state.timeline.get_display(*id));
		let boundary = self.boundary.flatten();
		let date = created(message.id);
		let new_day = match (previous.and_then(|m| created(m.id)), date) {
			(Some(a), Some(b)) => a.date() != b.date(),
			(None, Some(_)) => true,
			_ => false,
		};
		let grouped = previous.is_some_and(|previous| continues(previous, &message, boundary));
		let me = self.state.user.as_ref().map(|u| u.id);
		let mentioned =
			message.mention_everyone || message.mentions.iter().any(|u| Some(u.id) == me);
		let hovered = self.hovered == Some(message.id);
		let id = message.id;
		let name = self.state.message_author_name(&message).to_owned();
		let author_color = self
			.state
			.message_author_color(&message)
			.map_or(color(p.text_strong), |rgb| {
				color(ui::design::role_name_color(rgb, p.chat, p.text_strong))
			});
		let time = clock(message.id);
		let group: SharedString = format!("message-{id}").into();

		let mut content = div().flex_1().min_w_0().flex().flex_col();
		if !grouped {
			content = content.child(
				div()
					.h(px(22.))
					.flex()
					.items_center()
					.gap(px(8.))
					.child({
						let author = message.author.clone();
						let roles = message.author_roles.clone();
						let guild = self.state.channel(message.channel).and_then(|c| c.guild);
						div()
							.id(("author", id.0))
							.cursor_pointer()
							.hover(|d| d.underline())
							.text_size(px(15.5))
							.font_weight(FontWeight::MEDIUM)
							.text_color(author_color)
							.on_click(cx.listener(move |this, event: &ClickEvent, _, cx| {
								this.open_profile(
									author.clone(),
									guild,
									roles.clone(),
									event.position(),
									cx,
								)
							}))
							.child(name.clone())
					})
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
					}))
					.child(
						div()
							.text_size(px(12.))
							.text_color(color(p.muted))
							.child(time.clone()),
					),
			);
		}
		if message.is_system() {
			content = content.child(
				div()
					.text_color(color(p.muted))
					.child(message.display_text().into_owned()),
			);
		} else {
			if message.forwarded {
				content = content.child(
					div()
						.text_size(px(13.))
						.italic()
						.text_color(color(p.muted))
						.child("↪ Forwarded"),
				);
			}
			let editor = self
				.editing
				.as_ref()
				.filter(|(edit, _)| *edit == message.id)
				.map(|(_, editor)| editor.clone());
			let body = match editor {
				Some(editor) => vec![self.inline_editor(editor, cx)],
				None => self.body(&message, cx),
			};
			content = content.child(
				div()
					.flex()
					.flex_col()
					.gap_1()
					.when(message.forwarded, |d| {
						d.pl(px(16.)).border_l_3().border_color(color(p.selected))
					})
					.children(body),
			);
		}
		if message.edited && self.editing.as_ref().is_none_or(|(edit, _)| *edit != id) {
			content = content.child(
				div()
					.text_size(px(12.))
					.text_color(color(p.muted))
					.child("(edited)"),
			);
		}
		if message.unsupported || message.extra_content.any() {
			content = content.child(
				div()
					.text_size(px(13.))
					.text_color(color(p.muted))
					.child("Some content is only shown in the main Serein app."),
			);
		}
		content = content
			.children(
				message
					.attachments
					.iter()
					.enumerate()
					.map(|(ix, a)| self.attachment(a, ix, id)),
			)
			.children(
				(!message.embeds_suppressed)
					.then(|| message.embeds.iter().filter_map(|e| self.embed(e)))
					.into_iter()
					.flatten(),
			)
			.children(self.render_components(&message, cx))
			.children(self.reactions(&message, cx));

		let gutter = if grouped {
			div()
				.w(px(40.))
				.h(px(22.))
				.flex_none()
				.flex()
				.items_center()
				.justify_center()
				.text_size(px(11.))
				.text_color(gpui::transparent_black())
				.group_hover(group.clone(), |s| s.text_color(color(p.muted)))
				.child(time)
		} else {
			div()
				.flex_none()
				.mt(px(2.))
				.child(avatar(&name, 40., Some(&message.author)))
		};
		let row = div()
			.id(("message", id.0))
			.group(group)
			.relative()
			.w_full()
			.px_4()
			.pt(px(if grouped { 1. } else { 14. }))
			.pb(px(1.))
			.when(mentioned, |d| {
				d.bg(tint(p.warning, 0.10))
					.hover(|d| d.bg(tint(p.warning, 0.16)))
					.child(
						div()
							.absolute()
							.left_0()
							.top_0()
							.bottom_0()
							.w(px(3.))
							.bg(color(p.warning)),
					)
			})
			.when(!mentioned, |d| d.hover(|d| d.bg(tint(p.hover, 0.7))))
			.on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
				if *hovered {
					this.hovered = Some(id);
				} else if this.hovered == Some(id) {
					this.hovered = None;
				}
				cx.notify();
			}))
			.children(if grouped {
				None
			} else {
				self.reply_line(&message, cx)
			})
			.child(div().flex().gap(px(16.)).child(gutter).child(content))
			.when(hovered, |d| d.child(self.toolbar(&message, cx)));
		div()
			.w_full()
			.flex()
			.flex_col()
			.when(new_day, |d| {
				d.children(date.map(|at| Self::divider(date_label(at), false)))
			})
			.when(boundary == Some(id), |d| {
				d.child(Self::divider("New messages".into(), true))
			})
			.child(row)
			.when(ix + 1 == self.rows.len(), |d| d.pb(px(16.)))
			.into_any_element()
	}

	fn chat_header(&self, cx: &mut Context<Self>) -> impl IntoElement {
		let p = palette();
		let channel = self.state.selected.and_then(|id| self.state.channel(id));
		let (glyph, name) = match channel {
			Some(channel) if channel.guild.is_none() => (None, channel_label(channel)),
			Some(channel) => (
				Some(match channel.kind {
					10..=12 => Icon::Chats,
					5 => Icon::Megaphone,
					_ => Icon::Hash,
				}),
				channel_label(channel),
			),
			None => (Some(Icon::Hash), "Choose a conversation".into()),
		};
		div()
			.h(px(48.))
			.flex_none()
			.px_4()
			.border_b_1()
			.border_color(color(p.border))
			.flex()
			.items_center()
			.gap(px(8.))
			.child(match glyph {
				Some(glyph) => icon(glyph, px(22.), color(p.muted)).into_any_element(),
				None => avatar(
					&name,
					24.,
					channel.and_then(|c| c.recipients.first().filter(|_| c.recipients.len() == 1)),
				)
				.into_any_element(),
			})
			.child(
				div()
					.flex_1()
					.min_w_0()
					.overflow_hidden()
					.whitespace_nowrap()
					.text_ellipsis()
					.text_size(px(16.))
					.font_weight(FontWeight::SEMIBOLD)
					.text_color(color(p.text_strong))
					.child(name),
			)
			.when(self.state.selected.is_some(), |d| {
				let pins = self.state.search.as_ref().is_some_and(|view| view.pins);
				d.child(
					self.icon_button("pins-toggle", Icon::Pin, pins, "Pinned messages")
						.on_click(cx.listener(|this, _, _, cx| this.toggle_pins(cx))),
				)
			})
			.child(
				self.icon_button(
					"members-toggle",
					Icon::Users,
					self.members_open,
					"Show people",
				)
				.on_click(cx.listener(|this, _, _, cx| {
					this.members_open = !this.members_open;
					cx.notify();
				})),
			)
			.when(self.state.selected.is_some(), |d| {
				d.child(self.search_box())
			})
	}

	/// Shown while the first unread message is above the viewport, like the main app's bar.
	fn unread_banner(&self, cx: &mut Context<Self>) -> Option<impl IntoElement> {
		let p = palette();
		let first = self.first_rendered.replace(usize::MAX);
		let boundary = self.boundary.flatten()?;
		let index = self.rows.iter().position(|id| *id == boundary)?;
		// Rows are laid out after this runs, so this reads the previous frame; it redraws on scroll.
		if first == usize::MAX || index >= first {
			return None;
		}
		let count = self.rows.len() - index;
		Some(
			div()
				.id("unread-banner")
				.absolute()
				.top(px(8.))
				.left(px(16.))
				.right(px(16.))
				.h(px(32.))
				.px_3()
				.rounded(px(8.))
				.bg(color(p.accent))
				.shadow_md()
				.flex()
				.items_center()
				.justify_between()
				.cursor_pointer()
				.text_size(px(14.))
				.font_weight(FontWeight::MEDIUM)
				.text_color(color(p.accent_text))
				.on_click(cx.listener(move |this, _, _, cx| {
					this.messages.scroll_to_reveal_item(index);
					cx.notify();
				}))
				.child(format!(
					"{count} new message{}",
					if count == 1 { "" } else { "s" }
				))
				.child("Jump to unread ↑"),
		)
	}

	fn welcome(&self) -> impl IntoElement {
		let p = palette();
		let name = self
			.state
			.selected
			.and_then(|id| self.state.channel(id))
			.map(channel_label)
			.unwrap_or_default();
		let loading = self.state.history_pending;
		div()
			.size_full()
			.flex()
			.flex_col()
			.justify_end()
			.p_4()
			.gap_2()
			.when(loading, |d| {
				d.child(div().text_color(color(p.muted)).child("Loading messages…"))
			})
			.when(!loading && self.state.selected.is_some(), |d| {
				d.child(
					div()
						.size(px(68.))
						.rounded_full()
						.bg(color(p.raised))
						.flex()
						.items_center()
						.justify_center()
						.child(icon(Icon::Hash, px(40.), color(p.text_strong))),
				)
				.child(
					div()
						.text_size(px(28.))
						.font_weight(FontWeight::SEMIBOLD)
						.text_color(color(p.text_strong))
						.child(format!("Welcome to #{name}!")),
				)
				.child(
					div()
						.text_color(color(p.muted))
						.child(format!("This is the start of the #{name} channel.")),
				)
			})
	}

	fn composer_area(&self, cx: &mut Context<Self>) -> impl IntoElement {
		let p = palette();
		let can_send = self
			.state
			.selected
			.is_some_and(|channel| self.state.demo || self.state.can_send(channel));
		let reply = self
			.state
			.reply_target()
			.and_then(|id| self.state.timeline.get(id))
			.map(|message| self.state.message_author_name(message).to_owned());
		let pending = self
			.state
			.pending
			.iter()
			.filter(|pending| Some(pending.channel) == self.state.selected)
			.map(|pending| {
				div()
					.px_4()
					.pl(px(72.))
					.py_1()
					.text_color(color(p.muted))
					.overflow_hidden()
					.whitespace_nowrap()
					.text_ellipsis()
					.child(if pending.attachments.is_empty() {
						pending.content.clone()
					} else {
						// Uploads in flight name their files; progress shows in the composer.
						format!("{} [{}]", pending.content, pending.attachments.join(", "))
							.trim_start()
							.to_owned()
					})
			})
			.collect::<Vec<_>>();
		let typing = self.state.selected.and_then(|channel| {
			ui::typing_segments(&self.state, channel, std::time::Instant::now())
		});
		let tray = can_send.then(|| self.upload_tray(cx)).flatten();
		let joined = self.state.reply.is_some() || tray.is_some();
		div()
			.flex_none()
			.children(pending)
			.child(
				div()
					.relative()
					.px_4()
					.pt(px(2.))
					.children(self.render_picker(cx))
					.children(reply.map(|name| {
						div()
							.px_4()
							.pr_2()
							.py(px(5.))
							.rounded_t(px(8.))
							.bg(color(ui::design::mix(p.raised, p.base, 0.45)))
							.flex()
							.items_center()
							.child(
								div()
									.flex_1()
									.text_size(px(13.))
									.text_color(color(p.muted))
									.child("Replying to ")
									.child(
										div()
											.font_weight(FontWeight::SEMIBOLD)
											.text_color(color(p.text_strong))
											.child(name),
									)
									.flex()
									.gap_1(),
							)
							.child(
								self.icon_button(
									"cancel-reply",
									Icon::Close,
									false,
									"Cancel reply",
								)
								.size(px(22.))
								.on_click(cx.listener(|this, _, _, cx| {
									this.state.reply = None;
									cx.notify();
								})),
							)
					}))
					.children(tray.map(|tray| {
						div()
							.when(self.state.reply.is_none(), |d| d.rounded_t(px(8.)))
							.overflow_hidden()
							.child(tray)
					}))
					.child(
						div()
							.min_h(px(44.))
							.px(px(10.))
							.py(px(6.))
							.bg(color(p.raised))
							.when(joined, |d| d.rounded_b(px(8.)))
							.when(!joined, |d| d.rounded(px(8.)))
							.flex()
							.items_center()
							.gap(px(8.))
							.when(!can_send, |d| {
								d.child(div().flex_1().px_1().text_color(color(p.muted)).child(
									if self.state.selected.is_some() {
										"You do not have permission to send messages here."
									} else {
										"Choose a conversation to start chatting."
									},
								))
							})
							.when(can_send && self.can_attach_here(), |d| {
								d.child(
									self.icon_button(
										"attach",
										Icon::PlusCircle,
										false,
										"Upload a file",
									)
									.size(px(28.))
									.on_click(cx.listener(|this, _, _, cx| this.choose_files(cx))),
								)
							})
							.when(can_send, |d| {
								d.child(div().flex_1().min_w_0().child(self.composer.clone()))
									.child(
										self.icon_button("emoji", Icon::Smiley, false, "Emoji")
											.size(px(28.))
											.on_click(cx.listener(
												|this, event: &ClickEvent, window, cx| {
													let target = crate::emoji::Target::Composer;
													let at =
														event.position() - point(px(0.), px(20.));
													this.open_emoji_picker(target, at, window, cx)
												},
											)),
									)
									.child(
										self.icon_button("send", Icon::Send, true, "Send (Enter)")
											.size(px(28.))
											.on_click(cx.listener(|this, _, _, cx| this.send(cx))),
									)
							}),
					),
			)
			// The typing line keeps its height so the composer never jumps.
			.child(
				div()
					.h(px(24.))
					.px_4()
					.flex()
					.items_center()
					.text_size(px(12.))
					.text_color(color(p.muted))
					.overflow_hidden()
					.whitespace_nowrap()
					.children(typing.into_iter().flatten().map(|(text, strong)| {
						div()
							.when(strong, |d| {
								d.font_weight(FontWeight::SEMIBOLD)
									.text_color(color(p.text_strong))
							})
							.child(text)
					})),
			)
	}

	pub(crate) fn render_chat(&self, cx: &mut Context<Self>) -> impl IntoElement {
		let p = palette();
		div()
			.flex_1()
			.min_w_0()
			.h_full()
			.bg(color(p.chat))
			.flex()
			.flex_col()
			// Files dropped on the conversation join the composer's attachments.
			.when(self.can_attach_here(), |d| {
				d.drag_over::<ExternalPaths>(move |style, _, _, _| style.bg(tint(p.accent, 0.08)))
					.on_drop(cx.listener(|this, paths: &ExternalPaths, _, cx| {
						this.attach_paths(paths.paths().to_vec(), cx)
					}))
			})
			.child(self.chat_header(cx))
			.child(
				div()
					.flex_1()
					.min_h_0()
					.relative()
					.when(self.rows.is_empty(), |d| d.child(self.welcome()))
					.when(!self.rows.is_empty(), |d| {
						d.child(
							list(
								self.messages.clone(),
								cx.processor(|this, ix, _, cx| this.message(ix, cx)),
							)
							.size_full(),
						)
					})
					.children(self.unread_banner(cx))
					.when(
						!self.rows.is_empty() && self.messages.is_scrolled_to_end() == Some(false),
						|d| {
							d.child(
								div()
									.id("jump-to-present")
									.absolute()
									.right(px(16.))
									.bottom(px(12.))
									.size(px(44.))
									.rounded_full()
									.bg(color(p.accent))
									.shadow_lg()
									.flex()
									.items_center()
									.justify_center()
									.cursor_pointer()
									.hover(|d| d.opacity(0.9))
									.tooltip(tooltip("Jump to present"))
									.on_click(cx.listener(|this, _, _, cx| {
										this.messages.scroll_to_end();
										cx.notify();
									}))
									.child(icon(Icon::ArrowDown, px(20.), color(p.accent_text))),
							)
						},
					)
					.when(self.state.history_pending && !self.rows.is_empty(), |d| {
						d.child(
							div()
								.absolute()
								.top_2()
								.left_0()
								.right_0()
								.flex()
								.justify_center()
								.child(
									div()
										.px_3()
										.py_1()
										.rounded(px(12.))
										.bg(color(p.raised))
										.text_size(px(12.))
										.text_color(color(p.muted))
										.child("Loading earlier messages…"),
								),
						)
					}),
			)
			.children(self.interaction_notice())
			.child(self.composer_area(cx))
	}
}

#[cfg(test)]
mod tests {
	// Not a glob import: `gpui::*` would shadow the built-in `#[test]` attribute.
	use super::{GROUP_SECONDS, continues, custom_emoji_names, format_size, preview_text};
	use client_core::State;
	use model::{Id, Message};

	fn fixture() -> (State, Vec<Message>) {
		let state = test_support::chat_demo_state();
		let messages = state.timeline.iter().cloned().collect();
		(state, messages)
	}

	#[test]
	fn groups_same_author_within_five_minutes_but_not_across_boundaries() {
		let (_, messages) = fixture();
		assert!(continues(&messages[0], &messages[1], None));
		// Author change starts a new group.
		assert!(!continues(&messages[2], &messages[3], None));
		// The unread boundary always starts a group.
		assert!(!continues(&messages[0], &messages[1], Some(messages[1].id)));
		let mut later = messages[1].clone();
		later.id = Id(later.id.0 + ((GROUP_SECONDS as u64 * 1000) << 22));
		assert!(!continues(&messages[0], &later, None));
		// Replies always carry their own header and reply line.
		assert!(!continues(&messages[7], &messages[8], None));
	}

	#[test]
	fn reply_preview_resolves_mentions_and_channels() {
		let (state, messages) = fixture();
		let mut format = ui::FormatCache::default();
		// Friend nicknames win, as in the main app.
		let robin = state.user_display_name(&messages[6].mentions[0]);
		assert_eq!(
			preview_text(&mut format, &state, &messages[6]),
			format!("Hey @{robin} — see #long-form. This looks much easier to read.")
		);
		let mut long = messages[0].clone();
		long.content = "word ".repeat(100);
		assert_eq!(
			preview_text(&mut format, &state, &long).chars().count(),
			160
		);
	}

	#[test]
	fn inline_custom_emoji_read_as_names() {
		assert_eq!(
			custom_emoji_names("hi <:serein_wave:9001> and <a:party:42>!"),
			"hi :serein_wave: and :party:!"
		);
		// Malformed tokens stay literal.
		assert_eq!(
			custom_emoji_names("<:x:1> <:bad name:2>"),
			"<:x:1> <:bad name:2>"
		);
	}

	#[test]
	fn attachment_sizes_use_readable_units() {
		assert_eq!(format_size(512), "512 bytes");
		assert_eq!(format_size(2048), "2.0 KB");
		assert_eq!(format_size(3 * 1_048_576), "3.0 MB");
	}
}
