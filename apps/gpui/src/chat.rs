//! Conversation header, grouped timeline and composer, following the egui timeline's metrics.
use crate::sidebar::avatar;
use crate::theme::{FONT, Icon, color, icon, palette, solid, tint};
use crate::{Serein, channel_label, tooltip};
use client_core::State;
use gpui::{prelude::*, *};
use model::{Id, Message};
use std::ops::Range;
use std::sync::atomic::{AtomicBool, Ordering};

/// Consecutive messages from one author within this window share a header.
const GROUP_SECONDS: i64 = 300;
const BODY: f32 = 15.;
/// Inter's own line box (ascent + descent) at a size, as egui lays out a text row.
const LINE: f32 = 1.21;
/// Header row and compact-gutter height, as the main app's `MESSAGE_LINE`.
const MESSAGE_LINE: f32 = 22.;
/// The main app keeps this much space under the newest message for the typing overlay.
const TYPING_OVERLAY: f32 = 22.;
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
/// The main app's attachment sizes: bytes, then two decimals (none from 100 up).
fn format_size(bytes: u64) -> String {
	const UNITS: [&str; 4] = ["KB", "MB", "GB", "TB"];
	if bytes < 1024 {
		return format!("{bytes} bytes");
	}
	let mut value = bytes as f64 / 1024.;
	let mut unit = 0;
	while value >= 1024. && unit + 1 < UNITS.len() {
		value /= 1024.;
		unit += 1;
	}
	if value >= 100. {
		format!("{value:.0} {}", UNITS[unit])
	} else {
		format!("{value:.2} {}", UNITS[unit])
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
			.line_height(px(size * LINE))
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
		// The main app's text colours: strong text brighter, quotes and subtext muted.
		let mut foreground = if span.small || span.quote {
			p.muted
		} else if strong {
			p.text_strong
		} else {
			p.text
		};
		// User and channel mentions are links in the main app: pill colours, regular weight.
		let text: String = if let Some(user) = span.mention {
			foreground = p.mention_text;
			background = Some(color(p.mention_bg));
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
			background = Some(color(p.mention_bg));
			let role = self
				.state
				.channel(self.message.channel)
				.and_then(|c| c.guild)
				.and_then(|guild| self.state.guild_roles(guild))
				.and_then(|roles| roles.iter().find(|r| r.id == role))
				.ok_or(role);
			// Role pills wear the role's colour, kept readable on the pill.
			foreground = match role.as_ref().ok().map(|r| r.color).filter(|c| *c != 0) {
				Some(rgb) => ui::design::role_name_color(rgb, p.mention_bg, p.mention_text),
				None => p.mention_text,
			};
			match role {
				Ok(role) => format!("@{}", role.name),
				Err(id) => format!("@unknown-role ({id})"),
			}
		} else if let Some(channel) = span.channel {
			foreground = p.mention_text;
			background = Some(color(p.mention_bg));
			click = Some(Click::Channel(channel));
			match self.state.channel(channel) {
				Some(channel) => format!("#{}", channel_label(channel)),
				None => "#unknown-channel".into(),
			}
		} else if let Some((seconds, style)) = span.timestamp {
			background = Some(color(p.raised));
			ui::discord_timestamp(seconds, style).unwrap_or_else(|| span.text.to_owned())
		} else {
			if span.mass_mention {
				strong = true;
				foreground = p.mention_text;
				background = Some(color(p.mention_bg));
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
			background = Some(color(p.raised));
		}
		let mut hsla: Hsla = color(foreground).into();
		if span.spoiler && !self.revealed {
			background = Some(color(p.selected));
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

/// The main app's icon button: a `size` square with the glyph at 60%, a hover fill and a
/// brighter glyph on hover or while `active`; disabled buttons dim and ignore clicks.
pub(crate) fn tool(
	id: &'static str,
	glyph: Icon,
	size: f32,
	active: bool,
	enabled: bool,
	label: &'static str,
) -> Stateful<Div> {
	let p = palette();
	let group: SharedString = format!("tool-{id}").into();
	let tone = if !enabled {
		tint(p.muted, 0.5)
	} else if active {
		color(p.text_strong)
	} else {
		color(p.muted)
	};
	div()
		.id(id)
		.group(group.clone())
		.size(px(size))
		.flex_none()
		.rounded(px(6.))
		.flex()
		.items_center()
		.justify_center()
		.tooltip(tooltip(label))
		.when(enabled, |d| {
			d.cursor_pointer().hover(|d| d.bg(color(p.hover)))
		})
		.child(icon(glyph, px(size * 0.6), tone).when(enabled, |svg| {
			svg.group_hover(group, |s| s.text_color(color(p.text_strong)))
		}))
}

/// A custom emoji image once cached, else its `:name:` in muted text.
pub(crate) fn custom_emoji(id: Id, name: &str, size: f32) -> AnyElement {
	match crate::images::emoji(id) {
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

/// One-line text of a message with mentions resolved, for reply previews, and the byte ranges
/// drawn as mention pills.
fn preview_text(
	format: &mut ui::FormatCache,
	state: &State,
	message: &Message,
) -> (String, Vec<Range<usize>>) {
	let source = message.display_text();
	let mut text = String::new();
	let mut pills = Vec::new();
	for span in format.get(message.id, &source).spans() {
		if text.chars().count() >= 160 {
			break;
		}
		let start = text.len();
		if let Some(user) = span.mention {
			let name = message.mentions.iter().find(|u| u.id == user);
			text.push('@');
			text.push_str(name.map_or("unknown-user", |u| state.user_display_name(u)));
			pills.push(start..text.len());
		} else if let Some(channel) = span.channel {
			text.push('#');
			text.push_str(
				&state
					.channel(channel)
					.map_or_else(|| "unknown-channel".into(), channel_label),
			);
			pills.push(start..text.len());
		} else if span.role.is_some() {
			text.push_str("@role");
			pills.push(start..text.len());
		} else {
			text.extend(
				span.text
					.chars()
					.map(|c| if matches!(c, '\n' | '\r') { ' ' } else { c }),
			);
		}
	}
	let text: String = text.chars().take(160).collect();
	for pill in &mut pills {
		pill.end = pill.end.min(text.len());
	}
	pills.retain(|pill| pill.start < pill.end);
	(text, pills)
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
	let label = language_name(tag).map_or_else(|| tag.to_owned(), str::to_owned);
	// The main app's block: full width, 10x8 padding, a language header with a copy icon only
	// when the fence names one, and 14px monospace (body 15 * 0.9, rounded).
	div()
		.mt(px(4.))
		.w_full()
		.px(px(10.))
		.py(px(8.))
		.rounded(px(6.))
		.bg(color(p.raised))
		.border_1()
		.border_color(color(p.border))
		.flex()
		.flex_col()
		.gap(px(6.))
		.when(!label.is_empty(), |d| {
			d.child(
				div()
					.flex()
					.items_center()
					.justify_between()
					.pb(px(6.))
					.border_b_1()
					.border_color(color(p.border))
					.child(
						div()
							.text_size(px(12.))
							.font_weight(FontWeight::SEMIBOLD)
							.text_color(color(p.muted))
							.child(label),
					)
					.child(
						div()
							.id(ElementId::NamedInteger(
								format!("copy-code-{index}").into(),
								message.0,
							))
							.size(px(24.))
							.rounded(px(6.))
							.flex()
							.items_center()
							.justify_center()
							.cursor_pointer()
							.hover(|d| d.bg(color(p.hover)))
							.tooltip(tooltip("Copy code"))
							.on_click(cx.listener(move |this, _, _, cx| {
								cx.write_to_clipboard(ClipboardItem::new_string(copy.clone()));
								this.notify_user("Code copied");
								cx.notify();
							}))
							.child(icon(Icon::Copy, px(14.4), color(p.muted))),
					),
			)
		})
		.child(
			div()
				.text_size(px(14.))
				.line_height(px(16.8))
				.child(StyledText::new(code.to_owned()).with_runs(runs)),
		)
		.into_any_element()
}

/// The main app's display names for fenced-code language tags.
fn language_name(tag: &str) -> Option<&'static str> {
	Some(match tag.trim().to_ascii_lowercase().as_str() {
		"rs" | "rust" => "Rust",
		"js" | "javascript" | "jsx" | "mjs" | "cjs" | "node" => "JavaScript",
		"ts" | "typescript" | "tsx" | "mts" => "TypeScript",
		"py" | "python" | "python3" | "py3" => "Python",
		"go" | "golang" => "Go",
		"java" => "Java",
		"kt" | "kotlin" | "kts" => "Kotlin",
		"swift" => "Swift",
		"c" | "h" => "C",
		"cpp" | "c++" | "cc" | "cxx" | "hpp" | "hh" | "hxx" => "C++",
		"cs" | "csharp" | "c#" => "C#",
		"php" => "PHP",
		"rb" | "ruby" => "Ruby",
		"lua" => "Lua",
		"sh" | "bash" | "zsh" | "shell" | "fish" | "console" | "shellsession" => "Shell",
		"json" | "jsonc" | "json5" => "JSON",
		"yaml" | "yml" => "YAML",
		"toml" | "ini" | "cfg" => "TOML",
		"sql" | "mysql" | "postgres" | "postgresql" | "sqlite" | "psql" => "SQL",
		"html" | "htm" | "xml" | "svg" | "vue" | "xhtml" | "xaml" | "jsx-html" => "HTML",
		"css" | "scss" | "less" => "CSS",
		"diff" | "patch" => "Diff",
		"dart" => "Dart",
		"zig" => "Zig",
		_ => return None,
	})
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

/// A preview picture sized from its metadata so arrivals never move rows: fitted inside `bounds`
/// (never enlarged) with the main app's radius. Nothing when previews are off.
fn media_box(media: &model::EmbedMedia, bounds: (u32, u32), radius: f32) -> Option<Div> {
	if !crate::images::media_enabled() {
		return None;
	}
	let key = crate::images::media_key(media)?;
	let image = crate::images::media(&key);
	let (width, height) = crate::images::fit(media.width, media.height, bounds);
	Some(
		// The picture is laid out absolutely: in flow it would size the box by its own aspect.
		div()
			.flex_none()
			.relative()
			.w(px(width as f32))
			.h(px(height as f32))
			.max_w_full()
			.when(image.is_none(), |d| {
				d.rounded(px(radius)).bg(color(palette().canvas))
			})
			.children(image.map(|image| {
				img(image)
					.absolute()
					.inset_0()
					.size_full()
					.rounded(px(radius))
					.object_fit(ObjectFit::Contain)
			})),
	)
}

/// Only web addresses open from embeds, as in the main app.
fn external_url(url: &str) -> Option<String> {
	url::Url::parse(url)
		.ok()
		.filter(|url| matches!(url.scheme(), "https" | "http"))
		.map(|_| url.to_owned())
}

/// Discord link previews carry additional images as same-URL embed entries; like the main
/// app, only continuations without independent content join the first embed's gallery.
fn gallery_len(embeds: &[model::Embed]) -> usize {
	let Some(first) = embeds.first() else {
		return 0;
	};
	let eligible = |e: &model::Embed| {
		e.image.is_some()
			&& e.video.is_none()
			&& matches!(e.kind.as_str(), "rich" | "article" | "link" | "image")
	};
	if !eligible(first) || first.url.as_deref().is_none_or(str::is_empty) {
		return 1;
	}
	1 + embeds[1..]
		.iter()
		.take_while(|e| {
			eligible(e)
				&& e.url == first.url
				&& (e.title.is_none() || e.title == first.title)
				&& (e.description.is_none() || e.description == first.description)
				&& (e.author.is_none() || e.author == first.author)
				&& (e.provider.is_none() || e.provider == first.provider)
				&& (e.footer.is_none() || e.footer == first.footer)
				&& (e.timestamp.is_none() || e.timestamp == first.timestamp)
				&& (e.thumbnail.is_none() || e.thumbnail == first.thumbnail)
				&& (e.fields.is_empty() || e.fields == first.fields)
		})
		.count()
}

/// The main app's gallery mosaic: two columns with a 4px gap, three images as one tall tile
/// beside two stacked ones. Tiles crop to fill, with rounded top corners.
fn gallery(group: &[model::Embed], width: f32, message: Id, index: usize) -> AnyElement {
	let p = palette();
	let tile = |ix: usize| {
		let media = group[ix].image.as_ref();
		let image = media
			.filter(|_| crate::images::media_enabled())
			.and_then(crate::images::media_key)
			.and_then(|key| crate::images::media(&key));
		let target = media
			.and_then(|m| m.url.as_deref().or(m.proxy_url.as_deref()))
			.and_then(external_url);
		div()
			.id(ElementId::NamedInteger(
				format!("gallery-{index}-{ix}").into(),
				message.0,
			))
			.flex_1()
			.min_w_0()
			.h_full()
			.rounded_t(px(8.))
			.overflow_hidden()
			.bg(color(p.canvas))
			.when_some(target, |d, url| {
				d.cursor_pointer()
					.tooltip(tooltip("Open image…"))
					.on_click(move |_, window, cx| confirm_open(url.clone(), window, cx))
			})
			.children(image.map(|image| {
				img(image)
					.size_full()
					.rounded_t(px(8.))
					.object_fit(ObjectFit::Cover)
			}))
			.into_any_element()
	};
	let count = group.len();
	let mut grid = div().w(px(width)).max_w_full().flex().gap(px(4.));
	if count == 3 {
		grid = grid.aspect_ratio(1.).child(tile(0)).child(
			div()
				.flex_1()
				.min_w_0()
				.h_full()
				.flex()
				.flex_col()
				.gap(px(4.))
				.child(tile(1))
				.child(tile(2)),
		);
	} else {
		let rows = count.div_ceil(2);
		grid = grid
			.flex_col()
			.aspect_ratio(2. / rows as f32)
			.children((0..rows).map(|row| {
				div()
					.flex_1()
					.min_h_0()
					.flex()
					.gap(px(4.))
					.child(tile(row * 2))
					.child(if row * 2 + 1 < count {
						tile(row * 2 + 1)
					} else {
						div().flex_1().into_any_element()
					})
			}));
	}
	grid.into_any_element()
}

/// Gutter glyph and tint for a system message type, as the main app's system rows.
fn system_icon(kind: u8) -> (Icon, egui::Color32) {
	let p = palette();
	// Discord's boost pink; not part of any theme palette.
	let boost = egui::Color32::from_rgb(0xff, 0x73, 0xfa);
	match kind {
		1 | 7 => (Icon::ArrowRight, p.positive),
		2 => (Icon::ArrowLeft, p.danger),
		3 | 65 => (Icon::Phone, p.positive),
		4 => (Icon::Pencil, p.muted),
		5 => (Icon::Image, p.muted),
		6 => (Icon::Pin, p.muted),
		8..=11 => (Icon::Sparkle, boost),
		12 | 27..=31 => (Icon::Megaphone, p.muted),
		14 | 15 => (Icon::Compass, p.positive),
		16 | 17 => (Icon::Compass, p.warning),
		18 | 21 => (Icon::Thread, p.muted),
		22 => (Icon::UserPlus, p.muted),
		24 | 36 | 38 => (Icon::ShieldWarning, p.danger),
		37 | 39 | 62 => (Icon::ShieldWarning, p.positive),
		58 => (Icon::Trash, p.muted),
		59..=61 => (Icon::ShieldWarning, p.danger),
		55 => (Icon::ScreenShare, p.accent),
		67 => (Icon::Check, p.positive),
		25 | 26 | 32 => (Icon::Crown, p.warning),
		44 => (Icon::ShoppingCart, p.accent),
		46 => (Icon::ChartBar, p.muted),
		_ => (Icon::Help, p.muted),
	}
}

#[derive(Clone)]
enum SystemClick {
	Person(model::User),
	Thread(Id),
	Threads(Id),
}

impl Serein {
	/// The main app's system sentence: muted words, names in medium strong text that open
	/// profiles, a thread name that opens the thread, and the time after it.
	fn system_sentence(
		&self,
		message: &Message,
		system: &model::SystemMessage,
		cx: &mut Context<Self>,
	) -> AnyElement {
		let p = palette();
		let thread = (message.kind == 18 && system.content_shown)
			.then(|| self.state.thread_of(message).map(|c| c.id));
		let mut text = String::new();
		let mut runs = Vec::new();
		let mut ranges = Vec::new();
		let mut clicks = Vec::new();
		let mut push = |part: &str, strong: bool, click: Option<SystemClick>| {
			if part.is_empty() {
				return;
			}
			let start = text.len();
			text.push_str(part);
			runs.push(TextRun {
				len: part.len(),
				font: Font {
					weight: if strong {
						FontWeight::MEDIUM
					} else {
						FontWeight::NORMAL
					},
					..font(FONT)
				},
				color: color(if strong { p.text_strong } else { p.muted }).into(),
				background_color: None,
				underline: None,
				strikethrough: None,
			});
			if let Some(click) = click {
				ranges.push(start..text.len());
				clicks.push(click);
			}
		};
		for segment in &system.segments {
			let click = match (&segment.user, thread) {
				(Some(user), _) => Some(SystemClick::Person(user.clone())),
				(None, Some(Some(id))) if segment.strong => Some(SystemClick::Thread(id)),
				_ => None,
			};
			push(&segment.text, segment.strong, click);
		}
		if thread.is_some() {
			push(". See all ", false, None);
			push("threads", true, Some(SystemClick::Threads(message.channel)));
			push(".", false, None);
		}
		let guild = self.state.channel(message.channel).and_then(|c| c.guild);
		let view = cx.entity().downgrade();
		let sentence = InteractiveText::new(
			ElementId::NamedInteger("system".into(), message.id.0),
			StyledText::new(text).with_runs(runs),
		)
		.on_click(ranges, move |ix, window, cx| {
			let Some(click) = clicks.get(ix).cloned() else {
				return;
			};
			let position = window.mouse_position();
			let _ = view.update(cx, |this, cx| match click {
				SystemClick::Person(user) => {
					this.open_profile(user, guild, Vec::new(), position, cx)
				}
				SystemClick::Thread(id) => this.select(id, cx),
				SystemClick::Threads(parent) => this.open_threads(parent, cx),
			});
		});
		div()
			.flex()
			.flex_wrap()
			.items_baseline()
			.gap_x(px(8.))
			.line_height(px(BODY * LINE + 2.))
			.child(sentence)
			.child(
				div()
					.text_size(px(12.))
					.text_color(color(p.muted))
					.child(clock(message.id)),
			)
			.into_any_element()
	}
}

/// "Confirm before opening links", mirrored from the reading preferences.
static CONFIRM_LINKS: AtomicBool = AtomicBool::new(true);

pub(crate) fn set_confirm_links(on: bool) {
	CONFIRM_LINKS.store(on, Ordering::Relaxed);
}

/// HTTPS on Discord's own domains; like the main app, these open without asking.
fn discord_link(url: &str) -> bool {
	url::Url::parse(url).is_ok_and(|url| {
		url.scheme() == "https"
			&& url.port().is_none()
			&& url.host_str().is_some_and(|host| {
				[
					"discord.com",
					"discord.gg",
					"discordapp.com",
					"discordapp.net",
				]
				.iter()
				.any(|domain| {
					host == *domain
						|| host
							.strip_suffix(domain)
							.is_some_and(|prefix| prefix.ends_with('.'))
				})
			})
	})
}

/// Image and GIF embeds the main app shows as a bare preview, when this frontend can load it.
fn inline_image(embed: &model::Embed) -> Option<&model::EmbedMedia> {
	matches!(embed.kind.as_str(), "image" | "gifv")
		.then(|| embed.image.as_ref().or(embed.thumbnail.as_ref()))
		.flatten()
		.filter(|media| crate::images::media_key(media).is_some())
}

/// The message is nothing but links whose image or GIF previews are shown, as the main app's
/// `standalone_media_links`; spoilers and suppressed embeds keep the text.
fn standalone_media_links(message: &Message) -> bool {
	let attachment_spoilers = message.attachments.iter().any(|a| {
		a.spoiler
			|| a.filename.starts_with("SPOILER_")
			|| a.description.as_deref().is_some_and(|s| s.contains("||"))
	});
	let embed_spoilers = message.embeds.iter().any(|e| {
		[&e.title, &e.description]
			.into_iter()
			.flatten()
			.any(|s| s.contains("||"))
			|| e.fields
				.iter()
				.any(|f| f.name.contains("||") || f.value.contains("||"))
	});
	!message.embeds_suppressed
		&& !message.content.contains("||")
		&& !attachment_spoilers
		&& !embed_spoilers
		&& !message.content.trim().is_empty()
		&& message.content.split_whitespace().all(|link| {
			message.embeds.iter().any(|embed| {
				inline_image(embed).is_some()
					&& (embed.url.as_deref() == Some(link)
						|| [&embed.image, &embed.thumbnail]
							.into_iter()
							.flatten()
							.any(|media| media.url.as_deref() == Some(link)))
			})
		})
}

pub(crate) fn confirm_open_link(url: String, window: &mut Window, cx: &mut App) {
	confirm_open(url, window, cx)
}

fn confirm_open(url: String, window: &mut Window, cx: &mut App) {
	if !CONFIRM_LINKS.load(Ordering::Relaxed) || discord_link(&url) {
		cx.open_url(&url);
		return;
	}
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
		// The main app's divider spacing (16 + 4 plus its layout gaps), measured side by side.
		div()
			.mx_4()
			.mt(px(18.))
			.mb(px(9.))
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
				.h(px(18.))
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
			let (preview, pills) = preview_text(&mut self.format, &self.state, original);
			let (preview, pills) = if preview.is_empty() {
				("Click to see attachment".to_owned(), Vec::new())
			} else {
				(preview, pills)
			};
			// One line as in the main app: "@name  " in semibold, then the preview with pills.
			let lead = format!("@{name}  ");
			let run = |len: usize, weight: FontWeight, tone: egui::Color32, pill: bool| TextRun {
				len,
				font: Font {
					weight,
					..font(FONT)
				},
				color: color(tone).into(),
				background_color: pill.then(|| color(p.mention_bg).into()),
				underline: None,
				strikethrough: None,
			};
			let mut runs = vec![run(lead.len(), FontWeight::SEMIBOLD, p.muted, false)];
			let mut at = 0;
			for pill in &pills {
				if pill.start > at {
					runs.push(run(pill.start - at, FontWeight::NORMAL, p.muted, false));
				}
				runs.push(run(pill.len(), FontWeight::NORMAL, p.mention_text, true));
				at = pill.end;
			}
			if preview.len() > at {
				runs.push(run(preview.len() - at, FontWeight::NORMAL, p.muted, false));
			}
			div()
				.flex()
				.items_center()
				.gap(px(6.))
				.min_w_0()
				.child(avatar(&name, 16., Some(&original.author)))
				.child(
					div()
						.min_w_0()
						.overflow_hidden()
						.whitespace_nowrap()
						.text_ellipsis()
						.text_size(px(13.))
						.child(StyledText::new(format!("{lead}{preview}")).with_runs(runs)),
				)
				.into_any_element()
		} else {
			div()
				.text_size(px(13.))
				.text_color(color(p.muted))
				.child("Earlier message · View original")
				.into_any_element()
		};
		Some(
			div()
				.id(("reply-line", message.id.0))
				// The main app's reply row measures taller than its 18px spine.
				.h(px(26.))
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

	/// Attachments as the main app groups them: runs of images as a two-column mosaic, then
	/// video previews and file cards.
	fn attachments(&self, message: &Message) -> Vec<AnyElement> {
		let inline = |a: &model::Attachment| {
			a.is_image()
				&& !a.spoiler
				&& crate::images::media_enabled()
				&& crate::images::media_key(&a.media).is_some()
		};
		let mut output = Vec::new();
		let mut offset = 0;
		for group in message.attachments.chunk_by(|a, b| inline(a) == inline(b)) {
			if inline(&group[0]) {
				// Two columns of 207x180 cells in the main app's 420px box; one image gets 420x280.
				let (columns, cell) = if group.len() > 1 {
					(2, (207, 180))
				} else {
					(1, (420, 280))
				};
				let start = offset;
				output.push(
					div()
						.flex()
						.flex_col()
						.gap(px(6.))
						.children(group.chunks(columns).enumerate().map(|(row, images)| {
							div().flex().items_start().gap(px(6.)).children(
								images.iter().enumerate().filter_map(|(col, attachment)| {
									let ix = start + row * columns + col;
									let url = attachment.media.url.clone();
									let label = attachment
										.description
										.clone()
										.unwrap_or_else(|| attachment.filename.clone());
									media_box(&attachment.media, cell, 5.).map(|tile| {
										tile.id(ElementId::NamedInteger(
											format!("attachment-{ix}").into(),
											message.id.0,
										))
										.tooltip(tooltip(label))
										.when_some(url, |d, url| {
											d.cursor_pointer().on_click(move |_, window, cx| {
												confirm_open(url.clone(), window, cx)
											})
										})
									})
								}),
							)
						}))
						.into_any_element(),
				);
			} else {
				output.extend(group.iter().enumerate().map(|(ix, attachment)| {
					self.attachment(attachment, offset + ix, message.id)
						.into_any_element()
				}));
			}
			offset += group.len();
		}
		output
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
			(Icon::FileText, p.muted)
		} else {
			(Icon::File, p.muted)
		};
		let url = attachment.media.url.clone();
		// Videos play in the browser; the box keeps the metadata aspect so rows never jump.
		if attachment.is_video() && !attachment.spoiler {
			let media = &attachment.media;
			let (width, height) = crate::images::fit(media.width, media.height, (420, 236));
			let open = url.clone();
			return div()
				.id(ElementId::NamedInteger(
					format!("attachment-{ix}").into(),
					message.0,
				))
				.mb(px(6.))
				.w(px(width as f32))
				.max_w_full()
				.aspect_ratio(width as f32 / height as f32)
				.rounded(px(8.))
				.overflow_hidden()
				.bg(color(p.base))
				.border_1()
				.border_color(color(p.border))
				.relative()
				.flex()
				.items_center()
				.justify_center()
				.tooltip(tooltip("Play in browser"))
				.when_some(open, |d, url| {
					d.cursor_pointer()
						.on_click(move |_, window, cx| confirm_open(url.clone(), window, cx))
				})
				.child(
					div()
						.size(px(52.))
						.rounded_full()
						.bg(tint(egui::Color32::BLACK, 0.6))
						.flex()
						.items_center()
						.justify_center()
						.text_size(px(22.))
						.text_color(white())
						.child("▶"),
				)
				.child(
					div()
						.absolute()
						.left_0()
						.right_0()
						.bottom_0()
						.px_3()
						.py_1()
						.bg(tint(egui::Color32::BLACK, 0.5))
						.flex()
						.justify_between()
						.text_size(px(12.))
						.text_color(white())
						.child(
							div()
								.overflow_hidden()
								.whitespace_nowrap()
								.text_ellipsis()
								.child(attachment.filename.clone()),
						)
						.child(format_size(attachment.size)),
				);
		}
		// The main app's file card: 432px of content inside 12x10 padding.
		let demo = self.state.demo;
		div()
			.id(ElementId::NamedInteger(
				format!("attachment-{ix}").into(),
				message.0,
			))
			.mb(px(6.))
			.w(px(456.))
			.max_w_full()
			.px(px(12.))
			.py(px(10.))
			.rounded(px(8.))
			.bg(color(p.raised))
			.border_1()
			.border_color(color(p.border))
			.flex()
			.items_center()
			.gap(px(10.))
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
					.h(px(32.))
					.px(px(12.))
					.flex_none()
					.rounded(px(6.))
					.flex()
					.items_center()
					.text_size(px(14.))
					.font_weight(FontWeight::MEDIUM)
					// Synthetic files have nothing to fetch.
					.when(demo, |d| {
						d.text_color(tint(p.muted, 0.6))
							.tooltip(tooltip("Unavailable for synthetic attachments"))
					})
					.when(!demo, |d| {
						d.text_color(color(p.text))
							.cursor_pointer()
							.hover(|d| d.bg(color(p.hover)))
							.tooltip(tooltip("Open in browser"))
							.on_click(move |_, window, cx| confirm_open(url.clone(), window, cx))
					})
					.child("Download")
			}))
	}

	/// Embeds in the main app's order and style: bare image/GIF previews, galleries of
	/// same-link images, and rich cards with markdown descriptions and fields.
	fn embeds(&mut self, message: &Message, cx: &mut Context<Self>) -> Vec<AnyElement> {
		if message.embeds_suppressed {
			return Vec::new();
		}
		let mut output = Vec::new();
		let mut index = 0;
		while index < message.embeds.len() {
			let count = gallery_len(&message.embeds[index..]).max(1);
			let group = &message.embeds[index..index + count];
			if let Some(element) = self.embed(message, group, index, cx) {
				output.push(element);
			}
			index += count;
		}
		output
	}

	fn embed(
		&mut self,
		message: &Message,
		group: &[model::Embed],
		index: usize,
		cx: &mut Context<Self>,
	) -> Option<AnyElement> {
		let p = palette();
		let embed = &group[0];
		let id = |name: &str| {
			ElementId::NamedInteger(format!("embed-{index}-{name}").into(), message.id.0)
		};
		if let Some(media) = inline_image(embed) {
			if group.len() > 1 {
				return Some(
					div()
						.mb(px(6.))
						.child(gallery(group, 480., message.id, index))
						.into_any_element(),
				);
			}
			let target = embed
				.url
				.as_deref()
				.or(media.url.as_deref())
				.and_then(external_url);
			return media_box(media, (480, 320), 5.).map(|preview| {
				preview
					.id(id("media"))
					.mb(px(6.))
					.when_some(target, |d, url| {
						d.cursor_pointer()
							.tooltip(tooltip("Open image…"))
							.on_click(move |_, window, cx| confirm_open(url.clone(), window, cx))
					})
					.into_any_element()
			});
		}
		let link = |name: &str, label: String, url: Option<&str>, tone: egui::Color32| {
			let target = url.and_then(external_url);
			div()
				.id(id(name))
				.text_color(color(if target.is_some() { p.link } else { tone }))
				.when_some(target, |d, url| {
					d.cursor_pointer()
						.hover(|d| d.underline())
						.tooltip(tooltip("Open link…"))
						.on_click(move |_, window, cx| confirm_open(url.clone(), window, cx))
				})
				.child(label)
		};
		let small = |text: String| {
			div()
				.text_size(px(12.))
				.line_height(px(12. * LINE))
				.text_color(color(p.muted))
				.child(text)
		};
		// Parts after the body and component texts, so their format cache entries never collide.
		let part = 1000 + index as u16 * 64;
		let description = embed
			.description
			.clone()
			.map(|text| self.markdown(message, part, &text, cx));
		let fields = embed
			.fields
			.iter()
			.take(25)
			.enumerate()
			.map(|(ix, field)| {
				let value = self.markdown(message, part + 1 + ix as u16, &field.value, cx);
				(field.inline, field.name.clone(), value)
			})
			.collect::<Vec<_>>();
		// Up to three inline fields share a row in equal columns; others take the full width.
		let mut rows: Vec<Vec<(String, Vec<AnyElement>)>> = Vec::new();
		let mut row_inline = false;
		for (inline, name, value) in fields {
			match rows.last_mut() {
				Some(row) if inline && row_inline && row.len() < 3 => row.push((name, value)),
				_ => {
					rows.push(vec![(name, value)]);
					row_inline = inline;
				}
			}
		}
		let bar = embed
			.color
			.map_or(color(p.accent), |value| rgb(value & 0xff_ffff));
		let thumbnail = embed
			.thumbnail
			.as_ref()
			.and_then(|media| media_box(media, (84, 84), 5.));
		let video = embed.video.is_some() || matches!(embed.kind.as_str(), "video" | "gifv");
		let supported = matches!(
			embed.kind.as_str(),
			"rich" | "article" | "link" | "image" | "video" | "gifv"
		);
		Some(
			div()
				.w(px(480.))
				.max_w_full()
				.mb(px(6.))
				.relative()
				.p(px(12.))
				.rounded(px(5.))
				.bg(color(p.raised))
				.flex()
				.flex_col()
				.gap(px(4.))
				.child(
					div()
						.absolute()
						.left_0()
						.top(px(5.))
						.bottom(px(5.))
						.w(px(3.))
						.bg(bar),
				)
				.child(
					div()
						.flex()
						.items_start()
						.gap(px(16.))
						.child(
							div()
								.flex_1()
								.min_w_0()
								.flex()
								.flex_col()
								.gap(px(4.))
								.children(embed.provider.as_ref().map(|provider| {
									link(
										"provider",
										provider.name.clone(),
										provider.url.as_deref(),
										p.text,
									)
								}))
								.children(embed.author.as_ref().map(|author| {
									link(
										"author",
										author.name.clone(),
										author.url.as_deref(),
										p.text,
									)
								}))
								.children(embed.title.as_ref().map(|title| {
									link(
										"title",
										title.clone(),
										embed.url.as_deref(),
										p.text_strong,
									)
								}))
								.children(description.map(|paragraphs| {
									div().flex().flex_col().gap(px(4.)).children(paragraphs)
								})),
						)
						.children(thumbnail),
				)
				.children(rows.into_iter().map(|row| {
					div()
						.flex()
						.gap(px(16.))
						.children(row.into_iter().map(|(name, value)| {
							div()
								.flex_1()
								.min_w_0()
								.flex()
								.flex_col()
								.gap(px(4.))
								.child(div().text_color(color(p.text_strong)).child(name))
								.children(value)
						}))
				}))
				.when(group.len() > 1, |d| {
					d.child(gallery(group, 456., message.id, index))
				})
				.when(group.len() == 1, |d| {
					d.children(
						embed
							.image
							.as_ref()
							.and_then(|media| media_box(media, (456, 320), 5.)),
					)
				})
				.when(video, |d| {
					d.child(small(
						"Video preview · playback opens in your browser".into(),
					))
					.child(link(
						"video",
						"Open video…".into(),
						embed
							.url
							.as_deref()
							.or_else(|| embed.video.as_ref().and_then(|v| v.url.as_deref())),
						p.text,
					))
				})
				.when(
					!video && embed.title.is_none() && embed.url.is_some(),
					|d| {
						d.child(link(
							"source",
							"Open source…".into(),
							embed.url.as_deref(),
							p.text,
						))
					},
				)
				.children(
					embed
						.footer
						.as_ref()
						.map(|footer| small(footer.text.clone())),
				)
				.children(embed.timestamp.clone().map(small))
				.when(group.iter().any(|e| e.limited), |d| {
					d.child(small("Embed display limited".into()))
				})
				.when(!supported, |d| {
					d.child(small("Additional embed content is not supported".into()))
				})
				.into_any_element(),
		)
	}

	fn reactions(&self, message: &Message, cx: &mut Context<Self>) -> Option<impl IntoElement> {
		let p = palette();
		let reactions = message.reactions.as_ref().filter(|r| !r.is_empty())?;
		let id = message.id;
		Some(
			div()
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
								.hover(|d| d.bg(color(p.hover)))
						})
						.on_click(cx.listener(move |this, _, _, cx| {
							let command = this.state.prepare_reaction(id, emoji.clone());
							if command.is_none() && !this.state.demo {
								this.notify_user("Reactions need a live, fully synced connection.");
							}
							this.dispatch(command);
							cx.notify();
						}))
						.tooltip(crate::tooltip("Right-click to see who reacted"))
						.on_mouse_down(MouseButton::Right, {
							let emoji = reaction.emoji.clone();
							cx.listener(move |this, event: &MouseDownEvent, _, cx| {
								this.open_reactors(id, emoji.clone(), event.position, cx)
							})
						})
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
		let reply_target = self.state.reply_target() == Some(message.id);
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

		// The main app stacks a message's parts 4px apart; a lone line fills the 22px gutter.
		let system = message.system_message();
		let mut content = div()
			.flex_1()
			.min_w_0()
			.min_h(px(MESSAGE_LINE))
			// Measured from the main app: a headed message keeps 4px under its last line.
			.when(!grouped, |d| d.pb(px(4.)))
			.flex()
			.flex_col()
			.gap(px(4.));
		if !grouped && system.is_none() {
			content = content.child(
				div()
					.h(px(MESSAGE_LINE))
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
		if let Some(system) = &system {
			content = content.child(self.system_sentence(&message, system, cx));
		} else if message.is_system() {
			content = content.child(
				div()
					.text_color(color(p.muted))
					.child(message.display_text().into_owned()),
			);
		} else {
			let editor = self
				.editing
				.as_ref()
				.filter(|(edit, _)| *edit == message.id)
				.map(|(_, editor)| editor.clone());
			let body = match editor {
				Some(editor) => vec![self.inline_editor(editor, cx)],
				// "Hide image and GIF links": the previews below stand in for the text.
				None if self.settings.reading.hide_media_links
					&& crate::images::media_enabled()
					&& standalone_media_links(&message) =>
				{
					Vec::new()
				}
				None => self.body(&message, cx),
			};
			// The main app's order: body, embeds, attachments, then "(edited)" and the rest.
			// A forwarded payload sits behind a rail with its label, all indented together.
			let mut payload = body;
			payload.extend(self.embeds(&message, cx));
			payload.extend(self.attachments(&message));
			if message.forwarded {
				content = content.child(
					div()
						.relative()
						.pl(px(16.))
						.flex()
						.flex_col()
						.gap(px(4.))
						.child(
							div()
								.absolute()
								.left_0()
								.top_0()
								.bottom_0()
								.w(px(3.))
								.rounded(px(2.))
								.bg(color(p.selected)),
						)
						.child(
							div()
								.text_size(px(13.))
								.italic()
								.text_color(color(p.muted))
								.child("\u{21aa} Forwarded"),
						)
						.children(payload),
				);
			} else {
				content = content.children(payload);
			}
		}
		if message.edited && self.editing.as_ref().is_none_or(|(edit, _)| *edit != id) {
			content = content.child(
				div()
					.text_size(px(12.))
					.text_color(color(p.muted))
					.child("(edited)"),
			);
		}
		// The main app's placeholders for content without a preview, with a way out.
		let unknown_system = message.unsupported && message.system_summary().is_none();
		let extra = &message.extra_content;
		let placeholders = [
			(
				unknown_system,
				format!(
					"Unsupported message type {} · Preview unavailable",
					message.kind
				),
			),
			(extra.poll, "Poll · Preview unavailable".to_owned()),
			// Stickers are not drawn here yet, so they count as unavailable too.
			(
				extra.sticker_items || extra.stickers || !message.sticker_items.is_empty(),
				"Sticker · Preview unavailable".to_owned(),
			),
			(
				(extra.components || extra.components_v2) && message.components.is_empty(),
				"Components · Preview unavailable".to_owned(),
			),
		];
		if placeholders.iter().any(|(shown, _)| *shown) {
			let link = self
				.state
				.channel(message.channel)
				.filter(|c| self.state.can_view(c.id))
				.map(|c| {
					format!(
						"{}/{}",
						crate::nav_menu::channel_link(c.guild, c.id),
						message.id
					)
				});
			content =
				content
					.children(placeholders.into_iter().filter(|(shown, _)| *shown).map(
						|(_, text)| {
							div()
								.text_size(px(12.))
								.text_color(color(p.muted))
								.child(text)
						},
					))
					.child(
						div()
							.id(("open-in-discord", id.0))
							.flex_none()
							.self_start()
							.h(px(32.))
							.px(px(12.))
							.rounded(px(8.))
							.bg(color(p.raised))
							.flex()
							.items_center()
							.text_size(px(14.))
							.text_color(color(if link.is_some() { p.text } else { p.muted }))
							.when_some(link, |d, url| {
								d.cursor_pointer().hover(|d| d.bg(color(p.hover))).on_click(
									move |_, window, cx| confirm_open(url.clone(), window, cx),
								)
							})
							.child("Open in Discord"),
					);
		}
		content = content
			.children(self.render_components(&message, cx))
			.children(self.reactions(&message, cx));

		let gutter = if let Some((glyph, tone)) = system.as_ref().map(|_| system_icon(message.kind))
		{
			div()
				.w(px(40.))
				.h(px(MESSAGE_LINE))
				.pt(px(3.))
				.flex_none()
				.flex()
				.items_center()
				.justify_center()
				.child(icon(glyph, px(18.), color(tone)))
		} else if grouped {
			div()
				.w(px(40.))
				.h(px(MESSAGE_LINE))
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
			.when(!mentioned && !reply_target, |d| {
				d.hover(|d| d.bg(tint(p.hover, 0.7)))
			})
			// The message being replied to stays marked in the accent, as in the main app.
			.when(reply_target, |d| d.bg(tint(p.accent, 0.25)))
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
			// Room under the newest message for the typing overlay, as the main app keeps.
			.when(ix + 1 == self.rows.len(), |d| d.pb(px(8. + TYPING_OVERLAY)))
			.into_any_element()
	}

	fn chat_header(&self, cx: &mut Context<Self>) -> impl IntoElement {
		let p = palette();
		let channel = self.state.selected.and_then(|id| self.state.channel(id));
		let (glyph, name) = match channel {
			Some(channel) if channel.guild.is_none() => (None, channel_label(channel)),
			Some(channel) => (
				Some(match channel.kind {
					2 | 13 => Icon::Speaker,
					5 => Icon::Megaphone,
					15 | 16 => Icon::Forum,
					10..=12 => Icon::Thread,
					_ => Icon::Hash,
				}),
				channel_label(channel),
			),
			None => (Some(Icon::Hash), "Choose a conversation".into()),
		};
		// The main app's tools, left to right: reload, threads, pins, people, then search.
		let tools = self.state.selected.map(|selected| {
			let reload = self.state.freshness != model::Freshness::Loading
				&& self.state.can_read_history(selected);
			let threads = channel
				.filter(|c| c.guild.is_some() && matches!(c.kind, 0 | 5 | 15 | 16))
				.map(|c| {
					(
						c.id,
						self.state.can_archive(c.id, model::archives::Kind::Public),
					)
				});
			let pins_open = self.state.search.as_ref().is_some_and(|view| view.pins);
			div()
				.flex()
				.items_center()
				.gap(px(4.))
				.child(
					tool("reload", Icon::Reload, 32., false, reload, "Reload history").when(
						reload,
						|d| {
							d.on_click(cx.listener(|this, _, _, cx| {
								let command = this.state.history(None);
								this.dispatch(Some(command));
								this.hold_read_ack = false;
								this.messages.scroll_to_end();
								cx.notify();
							}))
						},
					),
				)
				.children(threads.map(|(parent, allowed)| {
					let open = self.state.archives.is_some();
					tool("threads", Icon::Thread, 32., open, allowed, "Threads").when(
						allowed,
						|d| {
							d.on_click(
								cx.listener(move |this, _, _, cx| this.open_threads(parent, cx)),
							)
						},
					)
				}))
				.child({
					let enabled = self.state.can_search() || pins_open || self.state.demo;
					tool(
						"pins-toggle",
						Icon::Pin,
						32.,
						pins_open,
						enabled,
						"Pinned messages",
					)
					.when(enabled, |d| {
						d.on_click(cx.listener(|this, _, _, cx| this.toggle_pins(cx)))
					})
				})
				.child(
					tool(
						"members-toggle",
						Icon::Users,
						32.,
						self.members_open,
						true,
						"Show member list",
					)
					.on_click(cx.listener(|this, _, _, cx| {
						this.members_open = !this.members_open;
						cx.notify();
					})),
				)
				.child(div().ml(px(4.)).child(self.search_box()))
		});
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
			.children(tools)
	}

	/// The main app's unread bar: flush with the top of the conversation, rounded below, shown
	/// until the unread messages are read (arriving holds it until the reader scrolls down).
	fn unread_banner(&self, cx: &mut Context<Self>) -> Option<impl IntoElement> {
		let p = palette();
		let first = self.first_rendered.replace(usize::MAX);
		if !self.state.show_missed_banner() {
			return None;
		}
		let index = self
			.boundary
			.flatten()
			.and_then(|boundary| self.rows.iter().position(|id| *id == boundary));
		// Rows are laid out after this runs, so this reads the previous frame; it redraws on scroll.
		let at_end = self.messages.is_scrolled_to_end() == Some(true);
		let whole = first == 0 && at_end;
		if !self.hold_read_ack && (at_end || whole) {
			return None;
		}
		let jump = self.state.can_jump_unread() || index.is_some();
		let text = color(p.accent_text);
		Some(
			div()
				.absolute()
				.top_0()
				.left(px(16.))
				.right(px(16.))
				.h(px(28.))
				.px(px(12.))
				.rounded_b(px(8.))
				.bg(color(p.accent))
				.flex()
				.items_center()
				.justify_between()
				.text_size(px(13.))
				.font_weight(FontWeight::MEDIUM)
				.text_color(text)
				.child("Unread messages")
				.when(jump, |d| {
					d.child(
						div()
							.id("jump-to-unread")
							.flex()
							.items_center()
							.gap(px(4.))
							.cursor_pointer()
							.hover(|d| d.underline())
							.on_click(cx.listener(move |this, _, _, cx| {
								match index {
									Some(index) => this.messages.scroll_to_reveal_item(index),
									None => {
										let command = this.state.open_unread();
										this.dispatch(command);
									}
								}
								cx.notify();
							}))
							.child("Jump to unread")
							.child(icon(Icon::ArrowUp, px(14.), text)),
					)
				}),
		)
	}

	/// The main app's typing overlay: resting dots and names over the bottom of the timeline.
	fn typing_overlay(&self) -> Option<impl IntoElement> {
		let p = palette();
		let segments = self.state.selected.and_then(|channel| {
			ui::typing_segments(&self.state, channel, std::time::Instant::now())
		})?;
		let dot = || div().size(px(5.)).rounded_full().bg(tint(p.muted, 0.675));
		Some(
			div()
				.absolute()
				.left(px(16.))
				.right(px(16.))
				.bottom_0()
				.h(px(TYPING_OVERLAY))
				.flex()
				.items_center()
				.overflow_hidden()
				.whitespace_nowrap()
				.child(
					div()
						.flex_none()
						.ml(px(2.))
						.mr(px(8.))
						.flex()
						.gap(px(2.))
						.child(dot())
						.child(dot())
						.child(dot()),
				)
				.child(
					div()
						.min_w_0()
						.flex()
						.overflow_hidden()
						.text_ellipsis()
						.text_size(px(12.5))
						.text_color(color(p.muted))
						.children(segments.into_iter().map(|(text, strong)| {
							div()
								.when(strong, |d| {
									d.font_weight(FontWeight::SEMIBOLD)
										.text_color(color(p.text_strong))
								})
								.child(text)
						})),
				),
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
		let reply = self.state.reply.map(|reply| {
			let target = reply.target();
			let name = self
				.state
				.timeline
				.get(target)
				.map_or("an earlier message", |message| message.author.name.as_str())
				.to_owned();
			(target, name, reply.mention)
		});
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
		// The send button dims until there is something to send, as in the main app.
		let has_content =
			!self.composer.read(cx).value().trim().is_empty() || self.uploads.has_files();
		let tray = can_send.then(|| self.upload_tray(cx)).flatten();
		let slash = can_send.then(|| self.render_slash_options(cx)).flatten();
		let joined = self.state.reply.is_some() || tray.is_some() || slash.is_some();
		div()
			.flex_none()
			.children(pending)
			.children(self.render_ephemeral(cx))
			.child(
				div()
					.relative()
					.px_4()
					.pt(px(2.))
					.children(self.render_picker(cx))
					.children(reply.map(|(target, name, mention)| {
						let openable = self.state.can_open_reply_target(target);
						// The main app's reply cap: who, then View original, the ping switch
						// and Cancel on the right.
						div()
							.pl_4()
							.pr_2()
							.py(px(5.))
							.rounded_t(px(8.))
							.bg(color(ui::design::mix(p.raised, p.base, 0.45)))
							.flex()
							.items_center()
							.gap(px(6.))
							.child(
								div()
									.flex_1()
									.min_w_0()
									.flex()
									.text_size(px(13.))
									.text_color(color(p.muted))
									.child("Replying to\u{a0}")
									.child(
										div()
											.font_weight(FontWeight::SEMIBOLD)
											.text_color(color(p.text_strong))
											.child(name),
									),
							)
							.child(
								div()
									.id("view-original")
									.text_size(px(12.))
									.text_color(if openable {
										color(p.muted)
									} else {
										tint(p.muted, 0.5)
									})
									.when(openable, |d| {
										d.cursor_pointer()
											.hover(|d| d.text_color(color(p.text_strong)))
											.on_click(cx.listener(move |this, _, _, cx| {
												this.open_reply(target, cx)
											}))
									})
									.child("View original"),
							)
							.child(
								div()
									.id("reply-mention")
									.px(px(6.))
									.py(px(3.))
									.rounded(px(6.))
									.cursor_pointer()
									.hover(|d| d.bg(color(p.hover)))
									.text_size(px(12.))
									.font_weight(FontWeight::SEMIBOLD)
									.text_color(color(if mention { p.link } else { p.muted }))
									.tooltip(tooltip(if mention {
										"Click to disable pinging the original author."
									} else {
										"Click to enable pinging the original author."
									}))
									.on_click(cx.listener(|this, _, _, cx| {
										if let Some(reply) = this.state.reply.as_mut() {
											reply.mention = !reply.mention;
										}
										cx.notify();
									}))
									.child(if mention { "@ ON" } else { "@ OFF" }),
							)
							.child(
								tool(
									"cancel-reply",
									Icon::Close,
									22.,
									false,
									true,
									"Cancel reply",
								)
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
					.children(slash)
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
									tool(
										"attach",
										Icon::PlusCircle,
										28.,
										false,
										true,
										"Attach files",
									)
									.on_click(cx.listener(|this, _, _, cx| this.choose_files(cx))),
								)
							})
							.when(can_send, |d| {
								let gif_open = self.gif_picker.is_some();
								let emoji_open = self.emoji_picker.is_some();
								d.child(div().flex_1().min_w_0().child(self.composer.clone()))
									.child(
										div()
											.flex()
											.items_center()
											.gap(px(4.))
											.child(
												tool(
													"gif",
													Icon::Gif,
													28.,
													gif_open,
													true,
													"Send a GIF",
												)
												.on_click(cx.listener(
													|this, event: &ClickEvent, window, cx| {
														let at = event.position()
															- point(px(0.), px(20.));
														this.open_gif_picker(at, window, cx)
													},
												)),
											)
											.child(
												tool(
													"emoji",
													Icon::Smiley,
													28.,
													emoji_open,
													true,
													"Insert an emoji",
												)
												.on_click(cx.listener(
													|this, event: &ClickEvent, window, cx| {
														this.close_gif_picker(None, cx);
														let target = crate::emoji::Target::Composer;
														let at = event.position()
															- point(px(0.), px(20.));
														this.open_emoji_picker(
															target, at, window, cx,
														)
													},
												)),
											)
											.child(
												tool(
													"send",
													Icon::Send,
													28.,
													false,
													has_content,
													"Send message",
												)
												.when(has_content, |d| {
													d.on_click(
														cx.listener(|this, _, _, cx| this.send(cx)),
													)
												}),
											),
									)
							}),
					),
			)
			// The main app's composer inset: the input sits 8px above the window edge.
			.child(div().h(px(8.)))
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
					// Scrolling down at the newest message reads an arrival held unread.
					.on_scroll_wheel(cx.listener(|this, event: &ScrollWheelEvent, window, cx| {
						if this.hold_read_ack
							&& event.delta.pixel_delta(px(16.)).y < px(0.)
							&& this.messages.is_scrolled_to_end() == Some(true)
						{
							this.hold_read_ack = false;
							this.mark_read(window);
							cx.notify();
						}
					}))
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
					.child({
						// The main app's bottom fade into the composer, taller while someone types
						// so the indicator stays legible over the messages behind it.
						let typing = self.state.selected.is_some_and(|channel| {
							ui::typing_segments(&self.state, channel, std::time::Instant::now())
								.is_some()
						});
						let mut dense = color(p.chat);
						if self.messages.is_scrolled_to_end() != Some(false) {
							dense.a *= 0.88;
						}
						let mut clear = dense;
						clear.a = 0.;
						div()
							.absolute()
							.left_0()
							.right_0()
							.bottom_0()
							.h(px(if typing { TYPING_OVERLAY + 52. } else { 20. }))
							.bg(linear_gradient(
								180.,
								linear_color_stop(clear, 0.),
								linear_color_stop(dense, 1.),
							))
					})
					.children(self.typing_overlay())
					.children(self.unread_banner(cx))
					.when(
						!self.rows.is_empty() && self.messages.is_scrolled_to_end() == Some(false),
						|d| {
							// The main app's round control: raised with a border, accent while
							// unread messages wait below.
							let unread = self
								.state
								.selected
								.is_some_and(|c| self.state.missed(c) == Some(true));
							d.child(
								div()
									.id("jump-to-present")
									.absolute()
									.right(px(16.))
									.bottom(px(TYPING_OVERLAY + 10.))
									.size(px(38.))
									.rounded_full()
									.when(unread, |d| d.bg(color(p.accent)))
									.when(!unread, |d| {
										d.bg(solid(p.raised))
											.border_1()
											.border_color(color(p.border))
									})
									.shadow_md()
									.flex()
									.items_center()
									.justify_center()
									.cursor_pointer()
									.hover(|d| d.opacity(0.9))
									.tooltip(tooltip(if unread {
										"New messages below · jump to present"
									} else {
										"Jump to present"
									}))
									.on_click(cx.listener(|this, _, window, cx| {
										this.hold_read_ack = false;
										this.messages.scroll_to_end();
										this.mark_read(window);
										cx.notify();
									}))
									.child(icon(
										Icon::ArrowDown,
										px(18.),
										color(if unread { p.accent_text } else { p.text_strong }),
									)),
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
			.children(self.render_gif_picker(cx))
	}
}

#[cfg(test)]
mod tests {
	// Not a glob import: `gpui::*` would shadow the built-in `#[test]` attribute.
	use super::{
		GROUP_SECONDS, continues, custom_emoji_names, discord_link, format_size, preview_text,
		standalone_media_links,
	};
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
		let (text, pills) = preview_text(&mut format, &state, &messages[6]);
		assert_eq!(
			text,
			format!("Hey @{robin} — see #long-form. This looks much easier to read.")
		);
		let pills = pills.iter().map(|p| &text[p.clone()]).collect::<Vec<_>>();
		assert_eq!(pills, [format!("@{robin}").as_str(), "#long-form"]);
		let mut long = messages[0].clone();
		long.content = "word ".repeat(100);
		assert_eq!(
			preview_text(&mut format, &state, &long).0.chars().count(),
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
	fn only_https_discord_links_skip_the_confirmation() {
		assert!(discord_link("https://discord.com/channels/1/2/3"));
		assert!(discord_link("https://ptb.discord.com/invite/x"));
		assert!(discord_link("https://discord.gg/serein"));
		assert!(!discord_link("http://discord.com/channels/1/2"));
		assert!(!discord_link("https://discord.com:8443/"));
		assert!(!discord_link("https://notdiscord.com/"));
		assert!(!discord_link("https://discord.com.example.org/"));
	}

	#[test]
	fn media_links_hide_only_beside_their_shown_preview() {
		let mut message = test_support::message(1, Id(1));
		let proxy = "https://images-ext-1.discordapp.net/external/abcdefghijklmnopqrstuvwxyz/x.gif";
		message.content = "https://klipy.com/gifs/waving-lizard".into();
		message.embeds = vec![model::Embed {
			kind: "gifv".into(),
			url: Some(message.content.clone()),
			thumbnail: Some(model::EmbedMedia {
				proxy_url: Some(proxy.into()),
				..Default::default()
			}),
			..Default::default()
		}];
		assert!(standalone_media_links(&message));
		message.embeds_suppressed = true;
		assert!(!standalone_media_links(&message));
		message.embeds_suppressed = false;
		message.content.insert_str(0, "Hello! ");
		assert!(!standalone_media_links(&message));
		message.content = message.embeds[0].url.clone().unwrap();
		message.content.push_str(" ||spoiler||");
		assert!(!standalone_media_links(&message));
		message.content = message.embeds[0].url.clone().unwrap();
		// A preview this frontend cannot load keeps the link visible.
		message.embeds[0].thumbnail = Some(model::EmbedMedia {
			proxy_url: Some("https://example.com/x.gif".into()),
			..Default::default()
		});
		assert!(!standalone_media_links(&message));
		message.embeds[0].thumbnail = Some(model::EmbedMedia {
			proxy_url: Some(proxy.into()),
			..Default::default()
		});
		message.embeds[0].kind = "rich".into();
		assert!(!standalone_media_links(&message));
	}

	#[test]
	fn attachment_sizes_use_readable_units() {
		assert_eq!(format_size(512), "512 bytes");
		assert_eq!(format_size(2048), "2.00 KB");
		assert_eq!(format_size(3 * 1_048_576), "3.00 MB");
		assert_eq!(format_size(200 * 1024), "200 KB");
	}
}
