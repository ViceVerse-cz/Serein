//! Bounded native text formatting. No HTML renderer, image loader, or automatic URL access.
use egui::{FontId, Stroke, TextFormat, text::LayoutJob};
use model::Id;
use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag, TagEnd};
use std::collections::HashMap;
use unicode_segmentation::UnicodeSegmentation;

const MAX_INPUT: usize = 8192;
const MAX_EVENTS: usize = 512;
const MAX_DEPTH: usize = 16;
const MAX_LINKS: usize = 16;
const MAX_SPOILERS: u8 = 32;
const MAX_BLOCKS: usize = 32;

/// True when the raw source escapes the token at `at` with an odd run of backslashes.
fn escaped(source: &str, at: usize) -> bool {
	source[..at]
		.bytes()
		.rev()
		.take_while(|b| *b == b'\\')
		.count()
		% 2 == 1
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
struct Style {
	strong: bool,
	italic: bool,
	underline: bool,
	code: bool,
	strike: bool,
	quote: bool,
	/// Discord heading level 1–3; zero for body text.
	heading: u8,
	/// Discord `-# ` subtext: smaller, quieter body text.
	small: bool,
	link: Option<usize>,
	mention: Option<Id>,
	role: Option<Id>,
	role_color: Option<u32>,
	mass_mention: bool,
	channel: Option<Id>,
	/// Discord `<t:seconds[:style]>` reference: rendered fresh each frame, never at parse time.
	timestamp: Option<(i64, u8)>,
	no_autolink: bool,
	spoiler: Option<u8>,
	/// Fenced code block index; the block widget replaces these spans when shown.
	block: Option<u8>,
}

/// Split styled text into Unicode BiDi runs in visual order. The text inside each run stays in
/// logical order so egui's shaper can still join Arabic-family scripts correctly.
fn bidi_spans(spans: &[(String, Style)]) -> Option<(Vec<(String, Style)>, bool)> {
	if spans.iter().all(|(text, _)| text.is_ascii()) {
		return None;
	}
	let text: String = spans.iter().map(|(text, _)| text.as_str()).collect();
	let bidi = unicode_bidi::BidiInfo::new(&text, None);
	if !bidi.has_rtl() {
		return None;
	}
	let right_aligned = bidi
		.paragraphs
		.iter()
		.find(|paragraph| {
			text[paragraph.range.clone()]
				.chars()
				.any(|c| !c.is_whitespace())
		})
		.is_some_and(|paragraph| paragraph.level.is_rtl());
	let mut styled = Vec::with_capacity(spans.len());
	let mut start = 0;
	for (value, style) in spans {
		let end = start + value.len();
		styled.push((start..end, *style));
		start = end;
	}
	let mut visual: Vec<(String, Style)> = Vec::with_capacity(spans.len());
	for paragraph in &bidi.paragraphs {
		let (levels, runs) = bidi.visual_runs(paragraph, paragraph.range.clone());
		for run in runs {
			let rtl = levels.get(run.start).is_some_and(|level| level.is_rtl());
			let mut parts: Vec<_> = styled
				.iter()
				.filter_map(|(range, style)| {
					let start = range.start.max(run.start);
					let end = range.end.min(run.end);
					(start < end).then_some((start..end, *style))
				})
				.collect();
			if rtl {
				parts.reverse();
			}
			for (range, style) in parts {
				let value = text[range].to_owned();
				if let Some((last, last_style)) = visual.last_mut()
					&& *last_style == style
				{
					last.push_str(&value);
				} else {
					visual.push((value, style));
				}
			}
		}
	}
	Some((visual, right_aligned))
}
/// Emoji artwork is taller than the body font, so any line carrying it grows. Knowing this at
/// parse time lets every widget on a line reserve that height before the first one is placed.
fn has_artwork(spans: &[(String, Style)]) -> bool {
	spans.iter().any(|(text, style)| {
		if style.code {
			return false;
		}
		let mut offset = 0;
		while offset < text.len() {
			if crate::emoji::custom_prefix(&text[offset..]).is_some() {
				return true;
			}
			let len = text[offset..]
				.graphemes(true)
				.next()
				.expect("remaining text")
				.len();
			if crate::emoji::lookup(&text[offset..offset + len]).is_some() {
				return true;
			}
			offset += len;
		}
		false
	})
}
/// Discord draws a message that carries nothing but emoji at roughly three times the body
/// size. More than a couple of dozen of them stay inline, as they do in the official client.
const MAX_JUMBO: usize = 27;
fn only_emoji(spans: &[(String, Style)], blocks: &[CodeBlock], mentions: usize) -> bool {
	if !blocks.is_empty() || mentions > 0 {
		return false;
	}
	let mut count = 0;
	for (text, style) in spans {
		if style.code
			|| style.block.is_some()
			|| style.link.is_some()
			|| style.timestamp.is_some()
			|| style.channel.is_some()
			|| style.mention.is_some()
			|| style.role.is_some()
			|| style.mass_mention
			|| style.heading > 0
			|| style.small
			|| style.quote
		{
			return false;
		}
		let mut offset = 0;
		while offset < text.len() {
			if let Some((_, len)) = crate::emoji::custom_prefix(&text[offset..]) {
				count += 1;
				offset += len;
				continue;
			}
			let cluster = text[offset..]
				.graphemes(true)
				.next()
				.expect("remaining text");
			if crate::emoji::lookup(cluster).is_some() {
				count += 1;
			} else if !cluster.chars().all(char::is_whitespace) {
				return false;
			}
			offset += cluster.len();
		}
	}
	(1..=MAX_JUMBO).contains(&count)
}
/// One fenced block: its display text plus highlighting computed once at parse time.
pub struct CodeBlock {
	/// Sanitised fence info word, shown when no known language matches it.
	tag: String,
	language: Option<crate::highlight::Language>,
	/// Exact text for copying.
	code: String,
	/// Tab-expanded text when it differs from `code`; egui has no tab stops.
	display: Option<String>,
	/// Highlighting of the displayed text, computed once at parse time.
	segments: Vec<crate::highlight::Segment>,
}
pub struct Formatted {
	spans: Vec<(String, Style)>,
	blocks: Vec<CodeBlock>,
	mention_count: usize,
	/// Set when any span renders artwork, whose line is taller than the body font.
	artwork: bool,
	/// Set when the whole message is emoji, which Discord draws at a larger size.
	jumbo: bool,
	pub links: Vec<String>,
	pub limited: bool,
	pub spoilers: bool,
}

#[derive(Default)]
pub struct FormatCache {
	entries: HashMap<(Id, u16), (String, Formatted, u64)>,
	bytes: usize,
	clock: u64,
}
impl FormatCache {
	pub fn retain(&mut self, mut keep: impl FnMut(Id) -> bool) {
		self.entries.retain(|(id, _), (source, parsed, _)| {
			if keep(*id) {
				true
			} else {
				self.bytes -= source.capacity() + parsed.bytes();
				false
			}
		});
	}
	pub fn get(&mut self, id: Id, source: &str) -> &Formatted {
		self.get_part(id, 0, source)
	}
	pub fn get_part(&mut self, message: Id, part: u16, source: &str) -> &Formatted {
		let id = (message, part);
		self.clock += 1;
		if self
			.entries
			.get(&id)
			.is_some_and(|(cached, _, _)| cached == source)
		{
			let entry = self.entries.get_mut(&id).expect("cached message");
			entry.2 = self.clock;
		} else {
			if let Some((source, parsed, _)) = self.entries.remove(&id) {
				self.bytes -= source.capacity() + parsed.bytes();
			}
			let mut end = source.len().min(64 * 1024);
			while !source.is_char_boundary(end) {
				end -= 1;
			}
			let parsed = Formatted::parse(source);
			let source = source[..end].to_owned();
			self.bytes += source.capacity() + parsed.bytes();
			self.entries.insert(id, (source, parsed, self.clock));
			while self.entries.len() > 512 || self.bytes > 1024 * 1024 {
				let oldest = *self
					.entries
					.iter()
					.min_by_key(|(_, entry)| entry.2)
					.expect("cache over budget")
					.0;
				let (source, parsed, _) = self.entries.remove(&oldest).expect("oldest entry");
				self.bytes -= source.capacity() + parsed.bytes();
			}
		}
		&self.entries.get(&id).expect("one bounded message fits").1
	}
}

/// The URL passed to the OS is exactly the normalized target displayed for confirmation.
pub fn external_url(input: &str) -> Option<String> {
	if input.len() > 2048 || input.chars().any(|c| c.is_control() || c == '\\') {
		return None;
	}
	let url = url::Url::parse(input).ok()?;
	(matches!(url.scheme(), "https" | "http")
		&& url.host_str().is_some()
		&& url.username().is_empty()
		&& url.password().is_none())
	.then(|| url.to_string())
}

pub(super) fn discord_url(channel: &model::Channel, message: Option<Id>) -> Option<String> {
	if channel.id.0 == 0 || message.is_some_and(|id| id.0 == 0) {
		return None;
	}
	let scope = match channel.guild {
		Some(guild) if guild.0 != 0 && !matches!(channel.kind, 1 | 3) => guild.to_string(),
		None if matches!(channel.kind, 1 | 3) => "@me".into(),
		_ => return None,
	};
	let mut url = format!("https://discord.com/channels/{scope}/{}", channel.id);
	if let Some(message) = message {
		url.push('/');
		url.push_str(&message.to_string());
	}
	Some(url)
}

#[derive(Debug, PartialEq, Eq)]
pub(super) struct ChatLink {
	pub guild: Option<Id>,
	pub channel: Id,
	pub message: Option<Id>,
}

/// Recognize only Discord's chat routes; other destinations keep normal link handling.
pub(super) fn discord_chat_link(input: &str) -> Option<ChatLink> {
	let target = external_url(input)?;
	let url = url::Url::parse(&target).ok()?;
	if url.scheme() != "https"
		|| url.port().is_some()
		|| !matches!(
			url.host_str()?,
			"discord.com"
				| "www.discord.com"
				| "ptb.discord.com"
				| "canary.discord.com"
				| "discordapp.com"
				| "www.discordapp.com"
				| "ptb.discordapp.com"
				| "canary.discordapp.com"
		) {
		return None;
	}
	let id = |value: &str| {
		if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
			return None;
		}
		value.parse::<u64>().ok().filter(|id| *id != 0).map(Id)
	};
	let mut path = url.path().trim_end_matches('/').split('/');
	if !path.next()?.is_empty() || path.next()? != "channels" {
		return None;
	}
	let guild = match path.next()? {
		"@me" => None,
		value => Some(id(value)?),
	};
	let channel = id(path.next()?)?;
	let message = match path.next() {
		Some(value) => Some(id(value)?),
		None => None,
	};
	path.next().is_none().then_some(ChatLink {
		guild,
		channel,
		message,
	})
}

pub(super) fn confirm_external_link(
	ctx: &egui::Context,
	opening: &mut Option<String>,
	confirm_links: bool,
) {
	let Some(target) = opening.as_deref().and_then(external_url) else {
		*opening = None;
		return;
	};
	let discord = url::Url::parse(&target).is_ok_and(|url| {
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
	});
	if !confirm_links || discord {
		ctx.open_url(egui::OpenUrl::new_tab(target));
		*opening = None;
		return;
	}
	let mut confirm = false;
	let mut cancel = false;
	let response = crate::dialog::Dialog::new("confirm-external-link", "Open external link?")
		.subtitle("This destination opens in your default browser.")
		.width(460.0)
		.show(ctx, |d| {
			d.content(|ui| {
				let colors = crate::design::palette(ui);
				egui::Frame::new()
					.fill(colors.base)
					.stroke(egui::Stroke::new(1.0, colors.border))
					.corner_radius(8)
					.inner_margin(egui::Margin::symmetric(12, 10))
					.show(ui, |ui| {
						ui.set_width(ui.available_width());
						ui.add(
							egui::Label::new(egui::RichText::new(&target).monospace().size(13.0))
								.wrap()
								.selectable(true),
						);
					});
			});
			d.footer(|ui| {
				confirm =
					crate::dialog::action(ui, "Open in Browser", crate::dialog::Action::Primary)
						.clicked();
				cancel =
					crate::dialog::action(ui, "Cancel", crate::dialog::Action::Neutral).clicked();
			});
		});
	cancel |= response.close;
	if confirm && !cancel {
		// Revalidate the exact normalized destination shown above before emitting an OS action.
		if let Some(url) = external_url(&target) {
			ctx.open_url(egui::OpenUrl::new_tab(url));
		}
	}
	if confirm || cancel {
		*opening = None;
	}
}

/// Insert a newline before a closing ``` that ends a line of fenced content and after one that
/// is followed by more text, so CommonMark sees the fence Discord would. A fence whose opening
/// line already holds the closing run (` ```one line``` `) is left for the code-span path.
fn normalize_fences(input: &str) -> std::borrow::Cow<'_, str> {
	let bytes = input.as_bytes();
	let mut out: Option<String> = None;
	let mut open = false;
	let mut i = 0;
	let mut copied = 0;
	while let Some(offset) = input[i..].find("```") {
		let at = i + offset;
		let mut run_end = at;
		while run_end < bytes.len() && bytes[run_end] == b'`' {
			run_end += 1;
		}
		let line_end = input[run_end..]
			.find('\n')
			.map_or(input.len(), |n| run_end + n);
		if !open {
			if input[run_end..line_end].contains("```") {
				// Single-line fence pair: skip past the closing run on this line.
				let close = run_end + input[run_end..line_end].find("```").unwrap_or(0);
				let mut close_end = close;
				while close_end < bytes.len() && bytes[close_end] == b'`' {
					close_end += 1;
				}
				i = close_end;
				continue;
			}
			open = true;
			i = line_end;
			continue;
		}
		let line_start = input[..at].rfind('\n').map_or(0, |n| n + 1);
		let own_line = input[line_start..at].trim().is_empty();
		let trailing = input[run_end..line_end].trim().is_empty();
		if !own_line || !trailing {
			let out = out.get_or_insert_with(|| String::with_capacity(input.len() + 8));
			if !own_line {
				out.push_str(&input[copied..at]);
				out.push('\n');
				copied = at;
			}
			if !trailing {
				out.push_str(&input[copied..run_end]);
				out.push('\n');
				copied = run_end;
			}
		}
		open = false;
		i = run_end;
	}
	match out {
		Some(mut out) => {
			out.push_str(&input[copied..]);
			std::borrow::Cow::Owned(out)
		}
		None => std::borrow::Cow::Borrowed(input),
	}
}

/// Everything one message body needs while its spans are laid out, so a quote can lay out its
/// own nested run without repeating the argument list.
struct Render<'a> {
	opening: &'a mut Option<String>,
	users: &'a [model::User],
	source: Option<&'a crate::mentions::MentionSource<'a>>,
	profile: &'a mut Option<model::User>,
	channels: &'a [model::Channel],
	channel: &'a mut Option<Id>,
	guilds: &'a [model::Guild],
	roles: &'a [model::permissions::Role],
	images: &'a mut crate::avatars::Avatars,
	demo: bool,
	revealed: &'a mut u32,
	surface: &'a mut crate::select::Surface,
	query: &'a str,
	/// Row height reserved for artwork, so emoji and text share one baseline.
	line: Option<f32>,
}

/// Discord's quote rail and the gap between it and the quoted text.
const QUOTE_RAIL: i8 = 4;
const QUOTE_GAP: i8 = 8;

impl Formatted {
	pub fn parse(source: &str) -> Self {
		let mut end = source.len().min(MAX_INPUT);
		while !source.is_char_boundary(end) {
			end -= 1;
		}
		if let Some((line_end, _)) = source[..end].match_indices('\n').nth(127) {
			end = line_end;
		}
		// Discord closes a fence at the end of any line (` ```js\ncode``` `); CommonMark needs
		// the closing fence on its own line. Only inserted newlines differ from the source.
		let normalized = normalize_fences(&source[..end]);
		let input: &str = &normalized;
		let mut output = Self {
			spans: Vec::new(),
			blocks: Vec::new(),
			mention_count: 0,
			links: Vec::new(),
			limited: end < source.len(),
			spoilers: false,
			artwork: false,
			jumbo: false,
		};
		let mut stack = Vec::new();
		let mut style = Style::default();
		// Spoiler scope is independent of Markdown's style stack: emphasis and
		// link boundaries may start or end inside a concealed region.
		let mut open_spoiler: Option<(usize, u8)> = None;
		let mut regions = 0;
		// Discord semantics that CommonMark lacks: `>>> ` quotes the rest of the message,
		// `> ` quotes exactly one line, `-# ` marks one line as subtext, and blank lines
		// between blocks are kept instead of collapsed.
		let mut quote_all = false;
		let mut quote_lazy = false;
		let mut subtext = false;
		let mut block_end = 0;
		let mut lists: Vec<Option<u64>> = Vec::new();
		let line_prefix = |at: usize| -> &str {
			let line_start = input[..at].rfind('\n').map_or(0, |i| i + 1);
			&input[line_start..at]
		};
		let literal = |text: &str, range: &std::ops::Range<usize>| {
			text == &input[range.clone()]
				&& input[..range.start]
					.bytes()
					.rev()
					.take_while(|b| *b == b'\\')
					.count() % 2 == 0
		};
		let mut events = Parser::new_ext(input, Options::ENABLE_STRIKETHROUGH)
			.into_offset_iter()
			.peekable();
		// Merge only unchanged source text. Entity/escape expansions stay separate and inert,
		// even when followed by an identical literal reference.
		let events = std::iter::from_fn(|| {
			let (event, mut range) = events.next()?;
			let event = match event {
				Event::Text(text) if literal(&text, &range) => {
					let mut joined = text.into_string();
					while let Some((Event::Text(next), next_range)) = events.peek() {
						if range.end != next_range.start || !literal(next, next_range) {
							break;
						}
						joined.push_str(next);
						range.end = next_range.end;
						events.next();
					}
					Event::Text(joined.into())
				}
				event => event,
			};
			Some((event, range))
		});
		for (count, (event, range)) in events.enumerate() {
			if count >= MAX_EVENTS || stack.len() > MAX_DEPTH {
				return Self::limited_literal(input, source.contains("||"));
			}
			style.spoiler = open_spoiler.map(|(_, region)| region);
			if quote_all {
				style.quote = true;
			} else if quote_lazy {
				style.quote = false;
			}
			style.small = subtext;
			if matches!(
				event,
				Event::Start(
					Tag::Paragraph
						| Tag::Heading { .. }
						| Tag::CodeBlock(_)
						| Tag::BlockQuote(_)
						| Tag::List(_) | Tag::Item
				) | Event::Rule
			) {
				// Source blank lines between blocks are kept. A block whose range excludes its
				// own line terminator (fenced code) contributes one newline that is not blank.
				if range.start > block_end && !output.spans.is_empty() {
					let mut blank = input[block_end..range.start].matches('\n').count();
					if blank > 0 && !input[..block_end].ends_with('\n') {
						blank -= 1;
					}
					// A blank line inside a `>>>` quote belongs to its rail, not to the text around it.
					for _ in 0..blank {
						output.push(
							"\n",
							Style {
								quote: style.quote,
								..Style::default()
							},
						);
					}
				}
				block_end = block_end.max(range.start);
			}
			match event {
				Event::Start(tag) => {
					stack.push(style);
					match tag {
						Tag::Strong if input[range.start..].starts_with("__") => {
							style.underline = true;
						}
						Tag::Strong => style.strong = true,
						Tag::Heading { level, .. } => {
							let level = level as u8;
							if level <= 3 {
								style.strong = true;
								style.heading = level;
							} else {
								// Discord has three heading levels; deeper markers stay literal.
								output.push(&"#".repeat(usize::from(level)), style);
								output.push(" ", style);
							}
						}
						Tag::Emphasis => style.italic = true,
						Tag::Strikethrough => style.strike = true,
						Tag::CodeBlock(CodeBlockKind::Fenced(info)) => {
							style.code = true;
							style.block = output.open_block(&info);
						}
						// Discord has no indented code blocks: four leading spaces stay prose.
						Tag::CodeBlock(CodeBlockKind::Indented) | Tag::Paragraph => {}
						Tag::BlockQuote(_) => {
							style.quote = true;
							if input[range.start..].starts_with(">>>") {
								quote_all = true;
							}
						}
						Tag::List(start) => lists.push(start),
						Tag::Item => {
							output.push(&"  ".repeat(lists.len().saturating_sub(1)), style);
							match lists.last_mut() {
								Some(Some(number)) => {
									output.push(&format!("{number}. "), style);
									*number = number.saturating_add(1);
								}
								_ => output.push("• ", style),
							}
						}
						Tag::Link { dest_url, .. } => {
							style.no_autolink = true;
							style.link = output.add_link(&dest_url);
						}
						Tag::Image { .. } => {
							style.no_autolink = true;
							output.push("[image: ", style);
						}
						_ => {}
					}
				}
				Event::End(tag) => {
					match tag {
						TagEnd::Paragraph
						| TagEnd::Heading(_)
						| TagEnd::CodeBlock
						| TagEnd::BlockQuote(_)
						| TagEnd::Item => {
							// Fenced code text carries its own terminator and a quote's inner
							// blocks end their lines; neither may add a blank line.
							if !(matches!(tag, TagEnd::CodeBlock | TagEnd::BlockQuote(_))
								&& output.line_start())
							{
								output.push("\n", style);
							}
							if let Some(block) = style.block {
								output.close_block(block);
							}
							subtext = false;
							block_end = block_end.max(range.end);
						}
						TagEnd::List(_) => {
							lists.pop();
							block_end = block_end.max(range.end);
						}
						TagEnd::Image => output.push("]", style),
						_ => {}
					}
					if matches!(tag, TagEnd::BlockQuote(_)) {
						quote_lazy = false;
					}
					style = stack.pop().unwrap_or_default();
				}
				Event::Text(text) => {
					let mut range = range;
					let mut text: &str = &text;
					if !style.code
						&& text.starts_with("-# ")
						&& literal(text, &range)
						&& line_prefix(range.start)
							.chars()
							.all(|c| c == '>' || c == ' ')
					{
						subtext = true;
						style.small = true;
						text = &text[3..];
						range.start += 3;
					}
					if style.code || text != &input[range.clone()] {
						output.push(text, style);
						if let Some(block) = style.block {
							output.blocks[usize::from(block)].code.push_str(text);
						}
					} else {
						// A raw-equal Text event can begin with one escaped character
						// followed by ordinary source text. Keep that first character
						// inert without suppressing later literal spoiler delimiters.
						let escaped = if literal(text, &range) {
							0
						} else {
							text.chars().next().map_or(0, char::len_utf8)
						};
						output.push(&text[..escaped], style);
						if !output.push_spoiler_literal(
							&text[escaped..],
							style,
							&mut open_spoiler,
							&mut regions,
						) {
							return Self::limited_literal(input, true);
						}
					}
				}
				Event::Html(text) | Event::InlineHtml(text) => {
					let inert = Style {
						no_autolink: true,
						..style
					};
					if literal(&text, &range) {
						if !output.push_spoiler_literal(
							&text,
							inert,
							&mut open_spoiler,
							&mut regions,
						) {
							return Self::limited_literal(input, true);
						}
					} else {
						output.push(&text, inert);
					}
				}
				// ` ```code``` ` on one line is a fence to Discord but a code span to CommonMark.
				Event::Code(text) if input[range.clone()].starts_with("```") => {
					let block = output.open_block("");
					let style = Style {
						code: true,
						block,
						..style
					};
					output.push(&text, style);
					if let Some(block) = block {
						output.blocks[usize::from(block)].code.push_str(&text);
						output.close_block(block);
					}
				}
				Event::Code(text) => output.push(
					&text,
					Style {
						code: true,
						..style
					},
				),
				Event::SoftBreak | Event::HardBreak => {
					subtext = false;
					if style.quote && !quote_all && !quote_lazy {
						// A `> ` quote covers one line; CommonMark's lazy continuation does not.
						quote_lazy = !input[range.end..].trim_start_matches(' ').starts_with('>');
					}
					output.push("\n", style);
				}
				Event::Rule => output.push("────────\n", style),
				_ => {}
			}
		}
		if let Some((opening, region)) = open_spoiler {
			if end < source.len() {
				// A closing delimiter may be outside our byte/line window.
				return Self::limited_literal(input, true);
			}
			// An unmatched opening delimiter is literal, including its contents.
			for (_, style) in &mut output.spans[opening..] {
				if style.spoiler == Some(region) {
					style.spoiler = None;
				}
			}
		}
		// Block endings separate content, but the final one must not add an empty chat line.
		for (text, _) in output.spans.iter_mut().rev() {
			text.truncate(text.trim_end_matches('\n').len());
			if !text.is_empty() {
				break;
			}
		}
		output.artwork = has_artwork(&output.spans);
		output.jumbo = only_emoji(&output.spans, &output.blocks, output.mention_count);
		output
	}
	fn limited_literal(input: &str, concealed: bool) -> Self {
		let mut formatted = Self {
			spans: vec![(
				input.to_owned(),
				Style {
					spoiler: concealed.then_some(0),
					..Default::default()
				},
			)],
			blocks: Vec::new(),
			mention_count: 0,
			links: Vec::new(),
			limited: true,
			spoilers: concealed,
			artwork: false,
			jumbo: false,
		};
		formatted.artwork = has_artwork(&formatted.spans);
		formatted.jumbo = only_emoji(&formatted.spans, &formatted.blocks, formatted.mention_count);
		formatted
	}
	fn push_spoiler_literal(
		&mut self,
		text: &str,
		mut style: Style,
		open: &mut Option<(usize, u8)>,
		regions: &mut u8,
	) -> bool {
		let mut consumed = 0;
		let mut search = 0;
		while let Some(offset) = text[search..].find("||") {
			let start = search + offset;
			search = start + 1;
			if text[..start]
				.bytes()
				.rev()
				.take_while(|byte| *byte == b'\\')
				.count() % 2 != 0
			{
				continue;
			}
			self.push_literal(&text[consumed..start], style);
			if let Some((opening, region)) = open.take() {
				// Retain an empty marker so an empty paired region still has a reveal control.
				self.spans[opening].0.clear();
				self.spans[opening].1.spoiler = Some(region);
				*regions += 1;
				self.spoilers = true;
				style.spoiler = None;
			} else {
				if *regions == MAX_SPOILERS {
					return false;
				}
				let opening = self.spans.len();
				self.push("||", style);
				*open = Some((opening, *regions));
				style.spoiler = Some(*regions);
			}
			consumed = start + 2;
			search = consumed;
		}
		self.push_literal(&text[consumed..], style);
		true
	}
	fn push_literal(&mut self, text: &str, style: Style) {
		if style.no_autolink {
			self.push(text, style);
		} else {
			self.push_mentions(text, style, text);
		}
	}
	fn add_link(&mut self, target: &str) -> Option<usize> {
		let url = external_url(target)?;
		if let Some(index) = self.links.iter().position(|existing| *existing == url) {
			return Some(index);
		}
		if self.links.len() >= MAX_LINKS {
			return None;
		}
		self.links.push(url);
		Some(self.links.len() - 1)
	}
	fn push_autolinks(&mut self, text: &str, style: Style) {
		let mut consumed = 0;
		let mut scanned = 0;
		for word in text.split_whitespace() {
			let start = scanned + text[scanned..].find(word).expect("word from source");
			scanned = start + word.len();
			let candidate = word.trim_start_matches(['(', '[', '{']);
			let mut target = candidate.trim_end_matches(['.', ',', ';', '!', '?', ']', '}']);
			while target.ends_with(')') && target.matches(')').count() > target.matches('(').count()
			{
				target = &target[..target.len() - 1];
			}
			if (target.starts_with("https://") || target.starts_with("http://"))
				&& let Some(link) = self.add_link(target)
			{
				let link_start = start + word.len() - candidate.len();
				self.push(&text[consumed..link_start], style);
				self.push(
					target,
					Style {
						link: Some(link),
						..style
					},
				);
				consumed = link_start + target.len();
			}
		}
		self.push(&text[consumed..], style);
	}
	fn push_mentions(&mut self, text: &str, style: Style, source: &str) {
		let mut consumed = 0;
		let mut raw_cursor = 0;
		for (start, _) in text.match_indices(['<', '@']) {
			if start < consumed {
				continue;
			}
			let reference = &text[start..];
			if let Some((seconds, kind, len)) = model::timestamp_prefix(reference) {
				let token = &text[start..start + len];
				let Some(raw_start) = source[raw_cursor..].find(token).map(|i| i + raw_cursor)
				else {
					continue;
				};
				raw_cursor = raw_start + len;
				if escaped(source, raw_start) {
					continue;
				}
				self.push_autolinks(&text[consumed..start], style);
				self.push(
					token,
					Style {
						timestamp: Some((seconds, kind)),
						..style
					},
				);
				consumed = start + len;
				continue;
			}
			let is_role = reference.starts_with("<@&");
			let (id, len, is_channel, mass_mention) =
				if let Some(len) = model::mass_mention_prefix(reference) {
					(None, len, false, true)
				} else {
					let is_channel = reference.starts_with("<#");
					let Some((id, len)) = (if is_channel {
						model::channel_mention_prefix(reference)
					} else if is_role {
						model::role_mention_prefix(reference)
					} else {
						model::user_mention_prefix(reference)
					}) else {
						continue;
					};
					(Some(id), len, is_channel, false)
				};
			if self.mention_count >= model::MAX_MENTIONS {
				self.limited = true;
				break;
			}
			let token = &text[start..start + len];
			let Some(raw_start) = source[raw_cursor..].find(token).map(|i| i + raw_cursor) else {
				continue;
			};
			raw_cursor = raw_start + len;
			if escaped(source, raw_start) {
				continue;
			}
			self.push_autolinks(&text[consumed..start], style);
			self.push(
				token,
				Style {
					mention: id.filter(|_| !is_channel && !is_role),
					role: id.filter(|_| is_role),
					mass_mention,
					channel: id.filter(|_| is_channel),
					..style
				},
			);
			self.mention_count += 1;
			consumed = start + len;
		}
		self.push_autolinks(&text[consumed..], style);
	}
	/// True when the message is only emoji, so the caller can draw it at the larger size.
	pub fn jumbo(&self) -> bool {
		self.jumbo
	}
	#[cfg(test)]
	pub fn show(&self, ui: &mut egui::Ui, opening: &mut Option<String>) {
		self.show_mentions(ui, opening, &[], &mut None);
	}
	#[cfg(test)]
	pub fn show_mentions(
		&self,
		ui: &mut egui::Ui,
		opening: &mut Option<String>,
		users: &[model::User],
		profile: &mut Option<model::User>,
	) {
		self.show_with_images(
			ui,
			opening,
			users,
			None,
			profile,
			(&mut crate::avatars::Avatars::default(), true, &[]),
		);
	}
	pub fn show_with_images(
		&self,
		ui: &mut egui::Ui,
		opening: &mut Option<String>,
		users: &[model::User],
		source: Option<&crate::mentions::MentionSource<'_>>,
		profile: &mut Option<model::User>,
		media: (&mut crate::avatars::Avatars, bool, &[model::Guild]),
	) {
		let (images, demo, guilds) = media;
		let mut revealed = u32::MAX;
		let mut surface = crate::select::Surface::new(ui, "body");
		self.show_references(
			ui,
			opening,
			users,
			source,
			profile,
			(&[], &mut None, guilds, &[]),
			(images, demo, &mut revealed),
			&mut surface,
		);
		surface.finish(ui);
	}
	#[allow(clippy::too_many_arguments)]
	pub fn show_references(
		&self,
		ui: &mut egui::Ui,
		opening: &mut Option<String>,
		users: &[model::User],
		source: Option<&crate::mentions::MentionSource<'_>>,
		profile: &mut Option<model::User>,
		references: (
			&[model::Channel],
			&mut Option<Id>,
			&[model::Guild],
			&[model::permissions::Role],
		),
		media: (&mut crate::avatars::Avatars, bool, &mut u32),
		surface: &mut crate::select::Surface,
	) {
		self.show_search(
			ui, opening, users, source, profile, references, media, surface, "",
		);
	}
	#[allow(clippy::too_many_arguments)]
	pub fn show_search(
		&self,
		ui: &mut egui::Ui,
		opening: &mut Option<String>,
		users: &[model::User],
		source: Option<&crate::mentions::MentionSource<'_>>,
		profile: &mut Option<model::User>,
		references: (
			&[model::Channel],
			&mut Option<Id>,
			&[model::Guild],
			&[model::permissions::Role],
		),
		media: (&mut crate::avatars::Avatars, bool, &mut u32),
		surface: &mut crate::select::Surface,
		query: &str,
	) {
		let (channels, channel, guilds, roles) = references;
		let (images, demo, revealed) = media;
		// Relative timestamps age without input; a coarse tick keeps them honest without a timer.
		if self
			.spans
			.iter()
			.any(|(_, style)| matches!(style.timestamp, Some((_, b'R'))))
		{
			ui.ctx()
				.request_repaint_after(std::time::Duration::from_secs(20));
		}
		// Wrapping rows only grow around widgets placed after the tallest one, so a
		// mention before an emoji would keep the body line height and ride above the
		// picture. A zero-width reservation gives every widget on the line the artwork
		// height first; text is then centred in the same row everywhere.
		let line = self.artwork.then(|| crate::emoji::inline_size(ui));
		let mut render = Render {
			opening,
			users,
			source,
			profile,
			channels,
			channel,
			guilds,
			roles,
			images,
			demo,
			revealed,
			surface,
			query,
			line,
		};
		self.show_run(ui, &self.spans, &mut render, false);
	}
	/// One wrapped paragraph flow. `quoted` marks the nested run inside a quote rail, where
	/// another rail would repeat the indent instead of ending it.
	fn show_run(
		&self,
		ui: &mut egui::Ui,
		spans: &[(String, Style)],
		render: &mut Render<'_>,
		quoted: bool,
	) {
		let line = render.line;
		ui.allocate_ui_with_layout(
			egui::vec2(ui.available_width(), 0.0),
			egui::Layout::left_to_right(egui::Align::Min).with_main_wrap(true),
			|ui| {
				ui.spacing_mut().item_spacing = egui::Vec2::ZERO;
				ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
				let reserve = |ui: &mut egui::Ui| {
					if let Some(height) = line {
						ui.allocate_space(egui::vec2(0.0, height));
					}
				};
				let mut start = 0;
				while start < spans.len() {
					// Discord's quote rail spans the whole block: a nested run keeps every
					// wrapped line inside the indent, and the rail is painted around it.
					if !quoted && spans[start].1.quote {
						let count = spans[start..]
							.iter()
							.take_while(|(_, style)| style.quote)
							.count();
						let mut quoted_spans = &spans[start..start + count];
						while quoted_spans
							.last()
							.is_some_and(|(text, _)| text.trim_matches('\n').is_empty())
						{
							quoted_spans = &quoted_spans[..quoted_spans.len() - 1];
						}
						if !quoted_spans.is_empty() {
							let colors = crate::design::palette(ui);
							let width = ui.max_rect().width();
							ui.allocate_ui_with_layout(
								egui::vec2(width, 0.0),
								egui::Layout::top_down(egui::Align::Min),
								|ui| {
									ui.set_width(width);
									let block = egui::Frame::new()
										.inner_margin(egui::Margin {
											left: QUOTE_RAIL + QUOTE_GAP,
											right: 0,
											top: 2,
											bottom: 2,
										})
										.show(ui, |ui| {
											self.show_run(ui, quoted_spans, render, true);
										});
									let rect = block.response.rect;
									ui.painter().rect_filled(
										egui::Rect::from_min_size(
											rect.left_top(),
											egui::vec2(f32::from(QUOTE_RAIL), rect.height()),
										),
										2.0,
										colors.selected,
									);
								},
							);
						}
						start += count;
						continue;
					}
					let spoiler = spans[start].1.spoiler;
					if let Some(region) = spoiler
						&& *render.revealed & (1_u32 << region) == 0
					{
						// Hidden text never reaches labels, selection, tooltips, links,
						// mention actions, accessibility values or emoji image requests.
						let count = spans[start..]
							.iter()
							.take_while(|(_, style)| style.spoiler == spoiler)
							.count();
						let response = ui
							.push_id(("spoiler", region), |ui| ui.button("Reveal spoiler"))
							.inner;
						render.surface.keep(&response);
						if response.clicked() {
							*render.revealed |= 1_u32 << region;
						}
						start += count;
						continue;
					}
					if spans[start].0.is_empty() {
						start += 1;
						continue;
					}
					if let Some(id) = spans[start].1.channel {
						reserve(ui);
						let colors = crate::design::palette(ui);
						if let Some(target) = render.channels.iter().find(|target| {
							target.id == id
								&& target.guild.is_some()
								&& matches!(target.kind, 0 | 5 | 10..=12 | 15 | 16)
						}) {
							let label = format!("#{}", target.name);
							let response = ui
								.add(egui::Link::new(
									egui::RichText::new(&label)
										.strong()
										.color(colors.mention_text)
										.background_color(colors.mention_bg),
								))
								.on_hover_text("Open channel");
							render.surface.keep(&response);
							response.widget_info(|| {
								egui::WidgetInfo::labeled(
									egui::Role::Link,
									ui.is_enabled(),
									format!("{label}, open channel"),
								)
							});
							if response.clicked() {
								*render.channel = Some(id);
							}
						} else if render.channels.iter().all(|target| target.id != id) {
							let label = "#unknown-channel";
							let response = ui
								.add(egui::Link::new(
									egui::RichText::new(label)
										.strong()
										.color(colors.mention_text)
										.background_color(colors.mention_bg),
								))
								.on_hover_text("Load channel");
							render.surface.keep(&response);
							response.widget_info(|| {
								egui::WidgetInfo::labeled(
									egui::Role::Link,
									ui.is_enabled(),
									"Unknown channel, load channel",
								)
							});
							if response.clicked() {
								*render.channel = Some(id);
							}
						} else {
							// Like fenced code, this label brings its own galley: let the
							// block register it in reading order.
							let (galley_pos, galley, response) = egui::Label::new(&spans[start].0)
								.selectable(true)
								.layout_in_ui(ui);
							let response = response.on_hover_text(
								"Channel unavailable or unsupported in this session",
							);
							render.surface.keep(&response);
							render.surface.embed(&response, galley_pos, galley);
						}
						start += 1;
						continue;
					}
					if let Some(id) = spans[start].1.mention {
						reserve(ui);
						let colors = crate::design::palette(ui);
						let user = crate::mentions::find_user(id, render.users, render.source);
						let label = crate::mentions::mention_label(id, render.users, render.source);
						let response = ui
							.add(egui::Link::new(
								egui::RichText::new(&label)
									.strong()
									.color(colors.mention_text)
									.background_color(colors.mention_bg),
							))
							.on_hover_text("Open user profile");
						render.surface.keep(&response);
						response.widget_info(|| {
							egui::WidgetInfo::labeled(
								egui::Role::Link,
								ui.is_enabled(),
								format!("{label}, user profile"),
							)
						});
						crate::profiles::arm_profile_opener(ui, &response);
						if response.clicked() {
							let opened = user.cloned().unwrap_or(model::User {
								id,
								name: format!("User {id}"),
								avatar: None,
								webhook: false,
								kind: Default::default(),
								discriminator: 0,
								primary_guild: None,
							});
							crate::profiles::toggle_profile(render.profile, &opened);
						}
						start += 1;
						continue;
					}
					if let Some(block) = spans[start].1.block {
						let count = spans[start..]
							.iter()
							.take_while(|(_, style)| style.block == Some(block))
							.count();
						// Fenced code is a block element inside this wrapping horizontal flow.
						// Explicit row boundaries make egui reserve its full painted height,
						// rather than placing the following paragraph back on the same row.
						ui.end_row();
						let code_rect = Self::show_code_block(
							ui,
							&self.blocks[usize::from(block)],
							block,
							render.surface,
						);
						ui.end_row();
						render.surface.exclude(code_rect);
						start += count;
						continue;
					}
					let target = spans[start].1.link;
					let count =
						spans[start..]
							.iter()
							.take_while(|(_, style)| {
								style.link == target
									&& style.spoiler == spoiler && style.mention.is_none()
									&& style.channel.is_none() && style.block.is_none()
							})
							.count();
					// The block widget already breaks the line: a paragraph's trailing newline
					// before it would otherwise add an empty row.
					let trimmed;
					let spans = if self
						.spans
						.get(start + count)
						.is_some_and(|(_, s)| s.block.is_some())
						&& spans[start + count - 1].0.ends_with('\n')
					{
						let mut copy = spans[start..start + count].to_vec();
						let last = &mut copy[count - 1].0;
						last.truncate(last.len() - 1);
						trimmed = copy;
						&trimmed[..]
					} else {
						&spans[start..start + count]
					};
					// Role pills share the surrounding text's galley, including wrapping and emoji heights.
					let resolved;
					let spans = if spans
						.iter()
						.any(|(_, style)| style.role.is_some() || style.timestamp.is_some())
					{
						resolved = spans
							.iter()
							.map(|(text, style)| {
								if let Some(id) = style.role {
									let name =
										render.roles.iter().find(|role| role.id == id).map_or_else(
											|| format!("unknown-role ({id})"),
											|role| role.name.clone(),
										);
									(
										format!("@{name}"),
										Style {
											mass_mention: true,
											role_color: render
												.roles
												.iter()
												.find(|role| role.id == id)
												.map(|role| role.color)
												.filter(|color| *color != 0),
											..*style
										},
									)
								} else if let Some((seconds, kind)) = style.timestamp {
									(
										crate::local_time::discord_timestamp(seconds, kind)
											.unwrap_or_else(|| text.clone()),
										*style,
									)
								} else {
									(text.clone(), *style)
								}
							})
							.collect::<Vec<_>>();
						&resolved[..]
					} else {
						spans
					};
					reserve(ui);
					if let Some(index) = target {
						let url = &self.links[index];
						let label: String = spans.iter().map(|(text, _)| text.as_str()).collect();
						let response = Self::show_emoji(
							spans,
							ui,
							true,
							render.images,
							render.demo,
							render.guilds,
							render.surface,
							render.query,
						)
						.on_hover_text(url);
						response.widget_info(|| {
							egui::WidgetInfo::labeled(egui::Role::Link, ui.is_enabled(), &label)
						});
						if response.clicked() {
							*render.opening = Some(url.clone());
						}
					} else {
						Self::show_emoji(
							spans,
							ui,
							false,
							render.images,
							render.demo,
							render.guilds,
							render.surface,
							render.query,
						);
					}
					start += count;
				}
			},
		);
	}
	/// Full-width framed block: optional language header with a copy control, then the
	/// highlighted, wrapped, selectable monospace text.
	fn show_code_block(
		ui: &mut egui::Ui,
		block: &CodeBlock,
		index: u8,
		surface: &mut crate::select::Surface,
	) -> egui::Rect {
		let colors = crate::design::palette(ui);
		let code_colors = crate::design::code_colors(ui);
		let width = ui.max_rect().width();
		let body = egui::TextStyle::Body.resolve(ui.style());
		let mono = FontId::monospace((body.size * 0.9).round().max(11.0));
		let label = block
			.language
			.map(crate::highlight::Language::name)
			.filter(|_| !block.tag.is_empty())
			.map_or_else(|| block.tag.clone(), str::to_owned);
		let display = block.display.as_deref().unwrap_or(&block.code);
		let plain = [(0, display.len() as u32, crate::highlight::Token::Plain)];
		let segments: &[crate::highlight::Segment] = if block.segments.is_empty() {
			&plain
		} else {
			&block.segments
		};
		let id = ui.scope_id().with(("code-block", index));
		ui.allocate_ui_with_layout(
			egui::vec2(width, 0.0),
			egui::Layout::top_down(egui::Align::Min),
			|ui| {
				ui.set_width(width);
				ui.add_space(4.0);
				let frame = egui::Frame::new()
					.fill(ui.visuals().code_bg_color)
					.stroke(Stroke::new(1.0, colors.border))
					.corner_radius(6)
					.inner_margin(egui::Margin::symmetric(10, 8));
				let response = frame.show(ui, |ui| {
					ui.set_width(ui.available_width());
					ui.spacing_mut().item_spacing = egui::vec2(6.0, 6.0);
					if !label.is_empty() {
						ui.horizontal(|ui| {
							ui.label(
								egui::RichText::new(&label)
									.small()
									.color(colors.muted)
									.family(crate::design::semibold_family(ui.ctx())),
							);
							ui.with_layout(
								egui::Layout::right_to_left(egui::Align::Center),
								|ui| {
									surface.keep(&Self::copy_button(ui, id, &block.code));
								},
							);
						});
						let separator = ui.available_rect_before_wrap();
						ui.painter().hline(
							separator.x_range(),
							separator.top(),
							Stroke::new(1.0, colors.border),
						);
						ui.add_space(2.0);
					}
					let mut job = LayoutJob::default();
					job.wrap.max_width = ui.available_width();
					for (start, end, token) in segments {
						job.append(
							&display[*start as usize..*end as usize],
							0.0,
							TextFormat {
								font_id: mono.clone(),
								color: code_colors.color(*token, colors.text),
								italics: *token == crate::highlight::Token::Comment,
								..Default::default()
							},
						);
					}
					if display.is_empty() {
						job.append(" ", 0.0, TextFormat::simple(mono.clone(), colors.muted));
					}
					// Selection is registered by the surrounding block in `Surface::finish`,
					// so a drag through the code selects only what the pointer crossed.
					let (galley_pos, galley, response) = egui::Label::new(job)
						.wrap()
						.selectable(true)
						.layout_in_ui(ui);
					surface.embed(&response, galley_pos, galley);
					response.widget_info(|| {
						egui::WidgetInfo::labeled(
							egui::Role::Label,
							ui.is_enabled(),
							format!(
								"Code block{}: {}",
								if label.is_empty() {
									String::new()
								} else {
									format!(" ({label})")
								},
								block.code
							),
						)
					});
				});
				if label.is_empty() {
					// No header to hold the control: float it over the corner while hovered.
					let rect = response.response.rect;
					let size = 24.0;
					let target = egui::Rect::from_min_size(
						rect.right_top() + egui::vec2(-size - 5.0, 5.0),
						egui::Vec2::splat(size),
					);
					let copied = Self::copied_recently(ui, id);
					if ui.rect_contains_pointer(rect) || copied {
						// The code text is painted later, with the block's selection, so the
						// floating control needs a layer of its own to stay on top of it.
						let mut child = ui.new_child(
							egui::UiBuilder::new()
								.max_rect(target)
								.layer_id(egui::LayerId::new(
									egui::Order::Middle,
									id.with("copy-layer"),
								))
								.layout(egui::Layout::left_to_right(egui::Align::Center)),
						);
						child.painter().rect_filled(target, 6, colors.raised);
						surface.keep(&Self::copy_button(&mut child, id, &block.code));
					}
				}
				ui.add_space(4.0);
			},
		)
		.response
		.rect
	}
	fn copied_recently(ui: &egui::Ui, id: egui::Id) -> bool {
		let now = ui.input(|input| input.time);
		ui.data(|data| data.get_temp::<f64>(id))
			.is_some_and(|at| now - at < 1.5)
	}
	fn copy_button(ui: &mut egui::Ui, id: egui::Id, code: &str) -> egui::Response {
		let copied = Self::copied_recently(ui, id);
		let (icon, label) = if copied {
			(crate::icons::Icon::Check, "Copied")
		} else {
			(crate::icons::Icon::Copy, "Copy code")
		};
		let response = crate::icons::button(ui, icon, 24.0, label);
		if response.clicked() {
			ui.ctx().copy_text(code.to_owned());
			let now = ui.input(|input| input.time);
			ui.data_mut(|data| data.insert_temp(id, now));
		}
		if copied {
			ui.ctx()
				.request_repaint_after(std::time::Duration::from_millis(200));
		}
		response
	}
	/// One galley per run: emoji occupy fixed-width slots inside the text layout, so rows
	/// holding artwork grow before any text on them is positioned. Separate widgets would
	/// leave text placed earlier on the row misaligned with text placed after the emoji.
	#[allow(clippy::too_many_arguments)]
	fn show_emoji(
		spans: &[(String, Style)],
		ui: &mut egui::Ui,
		link: bool,
		images: &mut crate::avatars::Avatars,
		demo: bool,
		guilds: &[model::Guild],
		surface: &mut crate::select::Surface,
		query: &str,
	) -> egui::Response {
		struct Inline {
			text: String,
			custom: Option<model::Id>,
			image: Option<egui::Image<'static>>,
		}
		let size = crate::emoji::inline_size(ui);
		let mut atlas = None;
		let body = egui::TextStyle::Body.resolve(ui.style());
		let mut job = LayoutJob::default();
		let source: String = spans.iter().map(|(text, _)| text.as_str()).collect();
		let bidi = bidi_spans(spans);
		let (spans, right_aligned) = bidi
			.as_ref()
			.map_or((spans, false), |(spans, right)| (spans.as_slice(), *right));
		job.halign = if right_aligned {
			egui::Align::RIGHT
		} else {
			egui::Align::LEFT
		};
		let mut inlines: Vec<Inline> = Vec::new();
		// Label overwrites the first section's leading space with the wrap indentation.
		job.append("", 0.0, Self::format(ui, &Style::default()));
		for (text, style) in spans {
			let format = Self::format(ui, style);
			let mut start = 0;
			let mut offset = 0;
			while offset < text.len() {
				let custom = (!style.code)
					.then(|| crate::emoji::custom_prefix(&text[offset..]))
					.flatten();
				let len = custom.map_or_else(
					|| {
						text[offset..]
							.graphemes(true)
							.next()
							.expect("remaining text")
							.len()
					},
					|(_, len)| len,
				);
				let cluster = &text[offset..offset + len];
				let cell = if custom.is_none() && !style.code {
					crate::emoji::lookup(cluster)
				} else {
					None
				};
				if cell.is_none() && custom.is_none() {
					offset += len;
					continue;
				}
				if offset > start {
					job.append(&text[start..offset], 0.0, format.clone());
				}
				// One blank glyph forms an unbroken inline slot; its
				// character is expanded to the wire text below so selection copies the original.
				job.append(" ", 0.0, crate::emoji::inline_format(ui, size, size));
				inlines.push(Inline {
					text: cluster.to_owned(),
					custom: custom.map(|(id, _)| id),
					image: cell.and_then(|cell| {
						atlas
							.get_or_insert_with(|| crate::emoji::atlas(ui.ctx()))
							.map(|atlas| crate::emoji::image_cell(atlas, cluster, cell, size))
					}),
				});
				offset += len;
				start = offset;
			}
			if start < text.len() {
				job.append(&text[start..], 0.0, format);
			}
		}
		if !query.is_empty() {
			let mut sections = Vec::new();
			for section in &job.sections {
				let mut start = section.byte_range.start;
				for (offset, matched) in job.text
					[section.byte_range.start.0..section.byte_range.end.0]
					.match_indices(query)
				{
					let from = section.byte_range.start + offset;
					let mut normal = section.clone();
					normal.byte_range = start..from;
					sections.push(normal);
					let mut highlighted = section.clone();
					highlighted.byte_range = from..from + matched.len();
					highlighted.format.background =
						egui::Color32::from_rgba_unmultiplied(200, 160, 30, 85);
					sections.push(highlighted);
					start = from + matched.len();
				}
				let mut tail = section.clone();
				tail.byte_range = start..section.byte_range.end;
				sections.push(tail);
			}
			job.sections = sections;
		}
		let label = egui::Label::new(job)
			.wrap()
			.halign(if right_aligned {
				egui::Align::RIGHT
			} else {
				egui::Align::LEFT
			})
			.selectable(false);
		let (pos, mut galley, mut response) = label.layout_in_ui(ui);
		response
			.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Label, ui.is_enabled(), &source));
		let mut slots: Vec<(usize, egui::Rect)> = Vec::new();
		if !inlines.is_empty() {
			let wrap = galley.job.wrap.max_width;
			let galley_mut = std::sync::Arc::make_mut(&mut galley);
			let mut next = 0;
			for placed in &mut galley_mut.rows {
				if next >= inlines.len()
					|| !placed
						.glyphs
						.iter()
						.any(|glyph| glyph.chr == ' ' && glyph.line_height == size)
				{
					continue;
				}
				let row = std::sync::Arc::make_mut(&mut placed.row);
				let mut glyphs = Vec::with_capacity(row.glyphs.len());
				for glyph in &row.glyphs {
					// Placeholders are the only glyphs with the artwork line height; a literal
					// space in message text keeps the body font's row height.
					if next >= inlines.len() || glyph.chr != ' ' || glyph.line_height != size {
						glyphs.push(*glyph);
						continue;
					}
					let index = next;
					next += 1;
					let left = glyph.pos.x;
					slots.push((
						index,
						egui::Rect::from_min_size(
							pos + placed.pos.to_vec2() + egui::vec2(left, 0.0),
							egui::vec2(size, row.size.y),
						),
					));
					// One hit target: selection endpoints never split a sequence or markup.
					for chr in inlines[index].text.chars() {
						let mut slot = *glyph;
						slot.chr = chr;
						slot.pos.x = left;
						slot.advance_width = size;
						glyphs.push(slot);
					}
				}
				row.glyphs = glyphs;
			}
			// Selection and copy read the galley's text: expose exactly the original characters.
			galley_mut.job = std::sync::Arc::new(LayoutJob::simple(
				source,
				body,
				ui.visuals().text_color(),
				wrap,
			));
		}
		let artwork = slots
			.iter()
			.map(|(index, rect)| {
				let inline = &inlines[*index];
				crate::select::Artwork {
					rect: *rect,
					image: match inline.custom {
						Some(id) => images.custom_image(ui.ctx(), id, size, demo),
						None => inline.image.clone(),
					},
				}
			})
			.collect();
		surface.run(ui, &response, pos, galley, artwork);
		if link {
			response = ui.interact(
				response.rect,
				response.id.with("link"),
				egui::Sense::click(),
			);
			surface.through(&response);
		}
		if ui.is_rect_visible(response.rect) {
			for (index, rect) in &slots {
				if !ui.is_rect_visible(*rect) {
					continue;
				}
				let inline = &inlines[*index];
				let image = match inline.custom {
					Some(id) => images.custom_image(ui.ctx(), id, size, demo),
					None => inline.image.clone(),
				};
				if !link {
					let hit = ui
						.interact(
							*rect,
							response.id.with(("emoji", *index, &inline.text)),
							egui::Sense::click(),
						)
						.on_hover_cursor(egui::CursorIcon::PointingHand)
						.on_hover_text(&inline.text);
					hit.widget_info(|| {
						egui::WidgetInfo::labeled(
							egui::Role::Button,
							ui.is_enabled(),
							format!("Show emoji details: {}", inline.text),
						)
					});
					crate::emoji_details::show(ui, &hit, &inline.text, image, guilds);
					surface.through(&hit);
				}
			}
			if link && response.hovered() {
				ui.set_cursor_icon(egui::CursorIcon::PointingHand);
			}
		}
		let slot_at = |point: egui::Pos2| {
			slots
				.iter()
				.find(|(_, rect)| rect.contains(point))
				.map(|(index, _)| *index)
		};
		if let Some(index) = response.hover_pos().and_then(slot_at) {
			response = response.on_hover_text_at_pointer(&inlines[index].text);
		}
		let menu = response.id.with("emoji-menu");
		if response.secondary_clicked() {
			let target = response
				.interact_pointer_pos()
				.and_then(slot_at)
				.map(|index| inlines[index].text.clone());
			ui.data_mut(|data| data.insert_temp(menu, target));
		}
		if let Some(text) = ui.data(|data| data.get_temp::<Option<String>>(menu).flatten()) {
			egui::Popup::context_menu(&response).id(menu).show(|ui| {
				if ui.button("Copy emoji").clicked() {
					ui.ctx().copy_text(text);
					ui.close();
				}
			});
		}
		response
	}
	fn push(&mut self, text: &str, style: Style) {
		if !text.is_empty() {
			self.spans.push((text.to_owned(), style));
		}
	}
	/// Register a fenced block for `info`; beyond the block budget the text stays inline code.
	fn open_block(&mut self, info: &str) -> Option<u8> {
		if self.blocks.len() >= MAX_BLOCKS {
			return None;
		}
		let tag: String = info
			.split_whitespace()
			.next()
			.unwrap_or_default()
			.chars()
			.filter(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '#' | '-' | '_' | '.'))
			.take(24)
			.collect();
		self.blocks.push(CodeBlock {
			language: crate::highlight::Language::from_tag(&tag),
			tag,
			code: String::new(),
			display: None,
			segments: Vec::new(),
		});
		Some((self.blocks.len() - 1) as u8)
	}
	fn close_block(&mut self, index: u8) {
		let block = &mut self.blocks[usize::from(index)];
		let trimmed = block.code.trim_end_matches(['\n', '\r']).len();
		block.code.truncate(trimmed);
		block.display = block
			.code
			.contains('\t')
			.then(|| block.code.replace('\t', "    "));
		let display = block.display.as_deref().unwrap_or(&block.code);
		block.segments = block
			.language
			.map(|language| crate::highlight::tokenize(language, display))
			.unwrap_or_default();
	}
	fn line_start(&self) -> bool {
		self.spans
			.iter()
			.rev()
			.find(|(text, _)| !text.is_empty())
			.is_none_or(|(text, _)| text.ends_with('\n'))
	}
	pub fn append_inline_preview(
		&self,
		job: &mut LayoutJob,
		ui: &egui::Ui,
		users: &[model::User],
		source: Option<&crate::mentions::MentionSource<'_>>,
		roles: &[model::permissions::Role],
		channels: &[model::Channel],
	) {
		let colors = crate::design::palette(ui);
		let muted = TextFormat {
			font_id: FontId::proportional(13.0),
			color: colors.muted,
			..Default::default()
		};
		let pill = TextFormat {
			font_id: FontId::new(13.0, crate::design::semibold_family(ui.ctx())),
			color: colors.mention_text,
			background: colors.mention_bg,
			..Default::default()
		};
		let mut remaining = 120;
		for (text, style) in &self.spans {
			if remaining == 0 {
				break;
			}
			if text.is_empty() {
				continue;
			}
			let (display, format) = if let Some(id) = style.mention {
				(
					crate::mentions::mention_label(id, users, source),
					pill.clone(),
				)
			} else if let Some(id) = style.role {
				let name = roles
					.iter()
					.find(|role| role.id == id)
					.map_or_else(|| format!("unknown-role ({id})"), |role| role.name.clone());
				let style = Style {
					mass_mention: true,
					role_color: roles
						.iter()
						.find(|role| role.id == id)
						.map(|role| role.color)
						.filter(|color| *color != 0),
					..Default::default()
				};
				let mut format = Self::format(ui, &style);
				format.font_id = pill.font_id.clone();
				(format!("@{name}"), format)
			} else if let Some(id) = style.channel {
				match channels.iter().find(|channel| channel.id == id) {
					Some(channel)
						if channel.guild.is_some()
							&& matches!(channel.kind, 0 | 5 | 10..=12 | 15 | 16) =>
					{
						(format!("#{}", channel.name), pill.clone())
					}
					Some(_) => (text.clone(), muted.clone()),
					None => ("#unknown-channel".into(), pill.clone()),
				}
			} else if let Some((seconds, kind)) = style.timestamp {
				(
					crate::local_time::discord_timestamp(seconds, kind)
						.unwrap_or_else(|| text.clone()),
					muted.clone(),
				)
			} else if style.mass_mention {
				(text.clone(), pill.clone())
			} else {
				(
					text.chars()
						.map(|c| if matches!(c, '\n' | '\r') { ' ' } else { c })
						.collect(),
					muted.clone(),
				)
			};
			let take: String = display.chars().take(remaining).collect();
			remaining -= take.chars().count();
			if !take.is_empty() {
				job.append(&take, 0.0, format);
			}
		}
	}
	pub fn bytes(&self) -> usize {
		self.spans.capacity() * size_of::<(String, Style)>()
			+ self.spans.iter().map(|(s, _)| s.capacity()).sum::<usize>()
			+ self.blocks.capacity() * size_of::<CodeBlock>()
			+ self
				.blocks
				.iter()
				.map(|b| {
					b.tag.capacity()
						+ b.code.capacity()
						+ b.display.as_ref().map_or(0, String::capacity)
						+ b.segments.capacity() * size_of::<crate::highlight::Segment>()
				})
				.sum::<usize>()
			+ self.links.capacity() * size_of::<String>()
			+ self.links.iter().map(String::capacity).sum::<usize>()
	}
	fn format(ui: &egui::Ui, style: &Style) -> TextFormat {
		let visuals = ui.visuals();
		let colors = crate::design::palette(ui);
		let body = egui::TextStyle::Body.resolve(ui.style());
		let color = if style.mass_mention {
			style.role_color.map_or(colors.mention_text, |rgb| {
				crate::design::role_name_color(rgb, colors.mention_bg, colors.mention_text)
			})
		} else if style.link.is_some() {
			visuals.hyperlink_color
		} else if style.strong {
			visuals.strong_text_color()
		} else if style.quote || style.small {
			visuals.weak_text_color()
		} else {
			visuals.text_color()
		};
		// Discord proportions: h1 1.5×, h2 1.25×, h3 1× (bold), subtext 0.8× body.
		let size = body.size
			* match style.heading {
				1 => 1.5,
				2 => 1.25,
				_ if style.small => 0.8,
				_ => 1.0,
			};
		TextFormat {
			valign: ui.text_valign(),
			font_id: if style.code {
				FontId::monospace(size)
			} else if style.strong || style.mass_mention {
				// egui has no synthetic bold: emphasis comes from the bundled heavier face.
				FontId::new(size, crate::design::semibold_family(ui.ctx()))
			} else {
				FontId::new(size, body.family)
			},
			color,
			background: if style.mass_mention {
				colors.mention_bg
			} else if style.timestamp.is_some() || style.code {
				visuals.code_bg_color
			} else {
				egui::Color32::TRANSPARENT
			},
			italics: style.italic,
			strikethrough: if style.strike {
				Stroke::new(1.0, color)
			} else {
				Stroke::NONE
			},
			underline: if style.link.is_some() || style.underline {
				Stroke::new(1.0, color)
			} else {
				Stroke::NONE
			},
			..Default::default()
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn bidi_runs_keep_each_script_logical_and_follow_paragraph_direction() {
		let style = Style::default();
		let (rtl, right) = bidi_spans(&[("مرحبا English!".into(), style)]).unwrap();
		assert!(right);
		assert_eq!(
			rtl.iter()
				.map(|(text, _)| text.as_str())
				.collect::<String>(),
			"!Englishمرحبا "
		);

		let (ltr, right) = bidi_spans(&[("English مرحبا!".into(), style)]).unwrap();
		assert!(!right);
		assert_eq!(
			ltr.iter()
				.map(|(text, _)| text.as_str())
				.collect::<String>(),
			"English مرحبا!"
		);
		assert!(bidi_spans(&[("English only".into(), style)]).is_none());
		assert!(bidi_spans(&[]).is_none());
		let ascii: String = (0..=127).map(char::from).collect();
		assert!(bidi_spans(&[(ascii, style), ("second span".into(), style)]).is_none());
		for text in ["\u{202e}English\u{202c}", "English \u{2067}مرحبا\u{2069}"] {
			assert!(bidi_spans(&[(text.into(), style)]).is_some());
		}
	}

	#[test]
	#[ignore = "release-only BiDi microbenchmark; run with --ignored --nocapture"]
	fn bidi_ascii_benchmark() {
		use std::{hint::black_box, time::Instant};
		const ITERATIONS: usize = 100_000;
		for (name, spans) in [
			(
				"ascii",
				vec![(
					"A typical message with plain English text and a link https://example.com."
						.repeat(4),
					Style::default(),
				)],
			),
			(
				"ascii_styled",
				vec![
					("A styled message ".repeat(8), Style::default()),
					(
						"with a bold section ".repeat(8),
						Style {
							strong: true,
							..Default::default()
						},
					),
				],
			),
			(
				"mixed_rtl",
				vec![("English مرحبا! ".repeat(8), Style::default())],
			),
		] {
			let mut samples = Vec::with_capacity(5);
			for run in 0..6 {
				let start = Instant::now();
				for _ in 0..ITERATIONS {
					black_box(bidi_spans(black_box(&spans)));
				}
				if run > 0 {
					samples.push(start.elapsed());
				}
			}
			samples.sort_unstable();
			println!(
				"{name}: {ITERATIONS} calls, median {:?}, samples {samples:?}",
				samples[2]
			);
		}
	}

	#[test]
	fn messages_made_only_of_emoji_are_drawn_larger() {
		for source in [
			"\u{1f600}",
			"\u{1f600} \u{1f389}\n\u{1f388}",
			"<:serein_wave:9001>",
			"**\u{1f600}**",
		] {
			assert!(Formatted::parse(source).jumbo(), "{source}");
		}
		for source in [
			"",
			"hi \u{1f600}",
			"`\u{1f600}`",
			"# \u{1f600}",
			"<@9001> \u{1f600}",
			"https://example.com \u{1f600}",
			"plain text",
			&"\u{1f600}".repeat(MAX_JUMBO + 1),
		] {
			assert!(!Formatted::parse(source).jumbo(), "{source}");
		}
	}

	#[test]
	fn wrapped_text_and_emoji_stay_inside_the_starting_margin() {
		let ctx = egui::Context::default();
		for source in [
			"Words break across a narrow conversation window.",
			"Words 😀 more words <:wave:9001> and 😀 again.",
			"😀😀😀😀😀😀😀😀😀",
		] {
			for width in [40.0, 80.0, 140.0] {
				let parsed = Formatted::parse(source);
				let output = ctx.run_ui(Default::default(), |ui| {
					ui.set_width(width);
					parsed.show(ui, &mut None);
				});
				let galley = output
					.shapes
					.iter()
					.find_map(|shape| {
						if let egui::Shape::Text(text) = &shape.shape
							&& text.galley.text() == source
						{
							Some(&text.galley)
						} else {
							None
						}
					})
					.expect("message galley");
				assert!(galley.rows.len() > 1);
				assert_eq!(
					galley
						.rows
						.iter()
						.map(|row| row.glyphs.len())
						.sum::<usize>(),
					source.chars().count(),
				);
				for row in &galley.rows {
					for glyph in &row.glyphs {
						assert!(row.pos.x + glyph.pos.x >= -0.5, "{source}: {glyph:?}");
						assert!(
							row.pos.x + glyph.max_x() <= width + 1.0,
							"{source}: {glyph:?}"
						);
					}
				}
				output.drop_without_applying_deltas();
			}
		}
	}

	#[test]
	fn discord_chat_links_validate_origin_route_and_ids() {
		for host in [
			"discord.com",
			"www.discord.com",
			"ptb.discord.com",
			"canary.discord.com",
			"discordapp.com",
			"www.discordapp.com",
			"ptb.discordapp.com",
			"canary.discordapp.com",
		] {
			assert_eq!(
				discord_chat_link(&format!("https://{host}/channels/1/2/3?jump=true#message")),
				Some(ChatLink {
					guild: Some(Id(1)),
					channel: Id(2),
					message: Some(Id(3))
				})
			);
		}
		assert_eq!(
			discord_chat_link("https://discord.com/channels/@me/2/"),
			Some(ChatLink {
				guild: None,
				channel: Id(2),
				message: None
			})
		);
		assert_eq!(
			discord_chat_link("HTTPS://DISCORD.COM:443/channels/1/2"),
			Some(ChatLink {
				guild: Some(Id(1)),
				channel: Id(2),
				message: None
			})
		);
		for target in [
			"http://discord.com/channels/1/2",
			"https://discord.com.evil.example/channels/1/2",
			"https://evil.discord.com/channels/1/2",
			"https://discord.gg/channels/1/2",
			"https://discord.com@evil.example/channels/1/2",
			"https://user@discord.com/channels/1/2",
			"https://discord.com:444/channels/1/2",
			"https://discord.com/invite/example",
			"https://discord.com/channels/1",
			"https://discord.com/channels/1/2/3/4",
			"https://discord.com/channels/0/2",
			"https://discord.com/channels/1/0",
			"https://discord.com/channels/1/2/0",
			"https://discord.com/channels/1/+2",
			"https://discord.com/channels/1/18446744073709551616",
			"https://discord.com/channels/1/%32",
			"https://discord.com/channels/1/2\n",
			"https://discord.com\\channels/1/2",
		] {
			assert!(discord_chat_link(target).is_none(), "{target}");
		}
		assert!(
			discord_chat_link(&format!(
				"https://discord.com/channels/1/2?{}",
				"x".repeat(2048)
			))
			.is_none()
		);
	}
	#[test]
	fn quote_rails_span_every_wrapped_line_of_their_block() {
		fn shapes(
			shape: &egui::Shape,
			rails: &mut Vec<egui::Rect>,
			texts: &mut Vec<(String, egui::Rect, usize)>,
		) {
			match shape {
				egui::Shape::Rect(rect) if rect.rect.width() == f32::from(QUOTE_RAIL) => {
					rails.push(rect.rect);
				}
				egui::Shape::Text(text) => texts.push((
					text.galley.job.text.clone(),
					text.galley.rect.translate(text.pos.to_vec2()),
					text.galley.rows.len(),
				)),
				egui::Shape::Vec(children) => {
					for shape in children {
						shapes(shape, rails, texts);
					}
				}
				_ => {}
			}
		}
		let parsed = Formatted::parse(
			"> quoted words that have to wrap over several lines inside a narrow message body\n\nafter",
		);
		assert!(
			!parsed
				.spans
				.iter()
				.any(|(text, _)| text.contains('\u{2502}'))
		);
		let ctx = egui::Context::default();
		crate::design::apply(&ctx);
		let output = ctx.run_ui(
			egui::RawInput {
				screen_rect: Some(egui::Rect::from_min_size(
					egui::Pos2::ZERO,
					egui::vec2(220.0, 300.0),
				)),
				..Default::default()
			},
			|ui| parsed.show(ui, &mut None),
		);
		let (mut rails, mut texts) = (vec![], vec![]);
		for shape in &output.shapes {
			shapes(&shape.shape, &mut rails, &mut texts);
		}
		output.drop_without_applying_deltas();
		let (_, quote, rows) = texts
			.iter()
			.find(|(text, _, _)| text.starts_with("quoted words"))
			.unwrap_or_else(|| panic!("Missing quote: {texts:?}"));
		assert!(
			*rows >= 3,
			"The quote must wrap for this test to mean anything"
		);
		assert_eq!(rails.len(), 1, "One rail for one quoted block: {rails:?}");
		let rail = rails[0];
		assert!(
			rail.top() <= quote.top() && rail.bottom() >= quote.bottom(),
			"The rail must cover every wrapped line: {rail:?} against {quote:?}"
		);
		assert!(
			rail.right() <= quote.left(),
			"The rail sits left of the quoted text: {rail:?} against {quote:?}"
		);
	}

	#[test]
	fn block_endings_do_not_leave_a_blank_final_line() {
		for (source, expected) in [
			("Hello", "Hello"),
			("**Hello**", "Hello"),
			("One\nTwo", "One\nTwo"),
			("One\n\nTwo", "One\n\nTwo"),
			("One\n\n\nTwo", "One\n\n\nTwo"),
			("text\n```\ncode\n```\n\nend", "text\ncode\n\nend"),
			("# Title\nbody", "Title\nbody"),
			("- a\n- b", "• a\n• b"),
			("1. a\n2. b", "1. a\n2. b"),
			("> quoted\nplain", "quoted\nplain"),
			("> one\n> two", "one\ntwo"),
			(">>> all\nof\n\nthis", "all\nof\n\nthis"),
			("-# small print", "small print"),
			("#### deep", "#### deep"),
			("```\none\ntwo\n```", "one\ntwo"),
			("[Link](https://example.org)", "Link"),
			("||Hidden||", "Hidden"),
			("", ""),
		] {
			let parsed = Formatted::parse(source);
			let text: String = parsed.spans.iter().map(|(text, _)| text.as_str()).collect();
			assert_eq!(text, expected, "{source:?}");
		}
	}
	#[test]
	fn discord_routes_use_only_valid_typed_ids() {
		let mut channel = model::Channel {
			id: Id(10),
			guild: Some(Id(20)),
			kind: 0,
			name: "https://malicious.invalid/secret".into(),
			parent_id: None,
			position: 0,
			recipients: vec![],
			member_list_id: None,
			message_count: None,
			icon: None,
			last_message: None,
		};
		assert_eq!(
			discord_url(&channel, None).as_deref(),
			Some("https://discord.com/channels/20/10")
		);
		for kind in [10, 11, 12, 13, 14, 15, 16, 255] {
			channel.kind = kind;
			assert_eq!(
				discord_url(&channel, Some(Id(30))).as_deref(),
				Some("https://discord.com/channels/20/10/30")
			);
		}
		for kind in [1, 3] {
			channel.kind = kind;
			assert!(
				discord_url(&channel, None).is_none(),
				"DMs cannot have a guild route"
			);
			channel.guild = None;
			assert_eq!(
				discord_url(&channel, Some(Id(30))).as_deref(),
				Some("https://discord.com/channels/@me/10/30")
			);
			channel.guild = Some(Id(20));
		}
		channel.kind = 0;
		channel.guild = None;
		assert!(discord_url(&channel, None).is_none());
		channel.guild = Some(Id(0));
		assert!(discord_url(&channel, None).is_none());
		channel.guild = Some(Id(u64::MAX));
		channel.id = Id(u64::MAX);
		assert_eq!(
			discord_url(&channel, Some(Id(u64::MAX))).unwrap(),
			format!("https://discord.com/channels/{0}/{0}/{0}", u64::MAX)
		);
		assert!(discord_url(&channel, Some(Id(0))).is_none());
		channel.id = Id(0);
		assert!(discord_url(&channel, None).is_none());
	}

	#[test]
	fn link_preferences_keep_validation_and_discord_host_boundaries() {
		for (target, confirm_links, opens) in [
			("https://discord.com/channels/@me/1", true, true),
			("https://discord.gg/example", true, true),
			("https://cdn.discordapp.com/attachments/example", true, true),
			("https://discord.com.evil.example/", true, false),
			("https://evildiscord.com/", true, false),
			("https://discord.com@evil.example/", false, false),
			("javascript:alert(1)", false, false),
			("https://example.com/", true, false),
			("https://example.com/", false, true),
		] {
			let ctx = egui::Context::default();
			let mut opening = Some(target.to_owned());
			let output = ctx.run_ui(Default::default(), |_| {
				confirm_external_link(&ctx, &mut opening, confirm_links);
			});
			assert_eq!(
				!output.platform_output.commands.is_empty(),
				opens,
				"{target}"
			);
			if opens {
				assert!(opening.is_none());
			}
			output.drop_without_applying_deltas();
		}
	}

	#[test]
	fn external_confirmation_requires_explicit_action_and_displays_emitted_target() {
		fn text_position(shape: &egui::Shape, label: &str) -> Option<egui::Pos2> {
			match shape {
				egui::Shape::Text(text) if text.galley.job.text == label => {
					Some(text.pos + text.galley.size() / 2.0)
				}
				egui::Shape::Vec(shapes) => {
					shapes.iter().find_map(|shape| text_position(shape, label))
				}
				_ => None,
			}
		}
		for action in ["Cancel", "Escape", "Open in Browser"] {
			let ctx = egui::Context::default();
			let normalized = "https://example.com/b%20c";
			let mut opening = Some("HTTPS://EXAMPLE.COM:443/a/../b c".into());
			let frame = |opening: &mut Option<String>, events| {
				ctx.run_ui(
					egui::RawInput {
						screen_rect: Some(egui::Rect::from_min_size(
							egui::Pos2::ZERO,
							egui::vec2(340.0, 420.0),
						)),
						events,
						..Default::default()
					},
					|_| confirm_external_link(&ctx, opening, true),
				)
			};
			let mut position = None;
			for pass in 0..3 {
				let mut output = frame(&mut opening, vec![]);
				output.textures_delta.clear();
				assert!(output.platform_output.commands.is_empty());
				assert!(opening.is_some());
				assert!(
					pass == 0
						|| output.shapes.iter().any(|shape| text_position(
							&shape.shape,
							normalized
						)
						.is_some())
				);
				position = output
					.shapes
					.iter()
					.find_map(|shape| text_position(&shape.shape, action));
				output.drop_without_applying_deltas();
			}
			let events = if action == "Escape" {
				vec![egui::Event::Key {
					key: egui::Key::Escape,
					physical_key: None,
					pressed: true,
					repeat: false,
					modifiers: egui::Modifiers::NONE,
				}]
			} else {
				let pos = position.expect("Visible confirmation control");
				vec![
					egui::Event::PointerMoved(pos),
					egui::Event::PointerButton {
						pos,
						button: egui::PointerButton::Primary,
						pressed: true,
						modifiers: egui::Modifiers::NONE,
					},
					egui::Event::PointerButton {
						pos,
						button: egui::PointerButton::Primary,
						pressed: false,
						modifiers: egui::Modifiers::NONE,
					},
				]
			};
			let output = frame(&mut opening, events);
			let opened: Vec<_> = output
				.platform_output
				.commands
				.iter()
				.filter_map(|command| match command {
					egui::OutputCommand::OpenUrl(url) => Some(url.url.as_str()),
					_ => None,
				})
				.collect();
			assert_eq!(
				opened,
				if action == "Open in Browser" {
					vec![normalized]
				} else {
					vec![]
				}
			);
			assert!(opening.is_none());
			output.drop_without_applying_deltas();
			let output = frame(&mut opening, vec![]);
			assert!(output.platform_output.commands.is_empty());
			output.drop_without_applying_deltas();
		}
		let ctx = egui::Context::default();
		let mut invalid = Some("javascript:alert(1)".into());
		let output = ctx.run_ui(Default::default(), |_| {
			confirm_external_link(&ctx, &mut invalid, true)
		});
		assert!(invalid.is_none());
		assert!(output.platform_output.commands.is_empty());
		output.drop_without_applying_deltas();
	}

	#[test]
	fn inline_spoilers_preserve_crossing_styles_and_ignore_nonliteral_delimiters() {
		let parsed = Formatted::parse(
			"**Visible ||secret** still hidden|| end ||<@42> <#43> [link](https://hidden.example) 👩🏽‍💻||.",
		);
		assert!(parsed.spoilers && !parsed.limited);
		assert!(
			parsed
				.spans
				.iter()
				.any(|(text, style)| text == "secret" && style.strong && style.spoiler == Some(0))
		);
		assert!(
			parsed
				.spans
				.iter()
				.any(|(text, style)| text.contains("still hidden")
					&& !style.strong
					&& style.spoiler == Some(0))
		);
		assert!(
			parsed
				.spans
				.iter()
				.any(|(_, style)| style.mention == Some(Id(42)) && style.spoiler == Some(1))
		);
		assert!(
			parsed
				.spans
				.iter()
				.any(|(_, style)| style.channel == Some(Id(43)) && style.spoiler == Some(1))
		);
		let visible: String = parsed
			.spans
			.iter()
			.filter(|(_, style)| style.spoiler.is_none())
			.map(|(text, _)| text.as_str())
			.collect();
		assert_eq!(visible.trim_end(), "Visible  end .");
		let crossing = Formatted::parse("||hidden **also hidden|| visible**");
		assert!(
			crossing
				.spans
				.iter()
				.any(|(text, style)| text == " visible" && style.strong && style.spoiler.is_none())
		);
		for source in [
			"`||code||`",
			"```\n||code||\n```",
			r"\|\|escaped\|\|",
			"&#124;&#124;entity&#124;&#124;",
			"||unmatched",
			"unmatched||",
		] {
			let parsed = Formatted::parse(source);
			assert!(!parsed.spoilers, "not a spoiler: {source}");
			assert!(
				parsed
					.spans
					.iter()
					.all(|(_, style)| style.spoiler.is_none())
			);
		}
		let mixed = Formatted::parse(r"\|\|literal\|\| then ||hidden `||code||`||");
		assert!(mixed.spoilers);
		assert!(
			mixed
				.spans
				.iter()
				.any(|(text, style)| style.code && text == "||code||" && style.spoiler == Some(0))
		);
		let unmatched = Formatted::parse("plain ||unmatched **bold**");
		assert_eq!(
			unmatched
				.spans
				.iter()
				.map(|(text, _)| text.as_str())
				.collect::<String>()
				.trim_end(),
			"plain ||unmatched bold"
		);
	}

	#[test]
	fn spoiler_limits_never_fall_back_to_visible_secret_text() {
		let exact = Formatted::parse(&"||x|| ".repeat(32));
		assert!(exact.spoilers && !exact.limited);
		assert!(
			exact
				.spans
				.iter()
				.any(|(_, style)| style.spoiler == Some(31))
		);
		for source in [
			"||x|| ".repeat(33),
			format!("visible ||{}", "s".repeat(MAX_INPUT)),
			format!("visible ||{}", "secret\n".repeat(130)),
			format!("||secret|| {}", "*a* ".repeat(MAX_EVENTS)),
			format!("{}||secret||", "> ".repeat(MAX_DEPTH + 2)),
		] {
			let parsed = Formatted::parse(&source);
			assert!(parsed.limited && parsed.spoilers);
			assert!(parsed.links.is_empty());
			assert!(
				parsed
					.spans
					.iter()
					.all(|(_, style)| style.spoiler == Some(0))
			);
			assert!(parsed.bytes() < 16 * 1024);
		}
	}

	#[test]
	fn concealed_regions_skip_actions_images_and_text_until_keyboard_or_pointer_reveal() {
		fn collect(shape: &egui::Shape, texts: &mut Vec<(String, egui::Rect)>) {
			match shape {
				egui::Shape::Text(text) => texts.push((
					text.galley.text().into(),
					egui::Rect::from_min_size(text.pos, text.galley.size()),
				)),
				egui::Shape::Vec(shapes) => shapes.iter().for_each(|shape| collect(shape, texts)),
				_ => {}
			}
		}
		let key = |key| egui::Event::Key {
			key,
			physical_key: None,
			pressed: true,
			repeat: false,
			modifiers: egui::Modifiers::NONE,
		};
		for (width, dark) in [(220.0, false), (700.0, true)] {
			let parsed = Formatted::parse(
				"Visible ||secret <@42> <#43> [hidden link](https://hidden.example) <:wave:9001>|| end",
			);
			let ctx = egui::Context::default();
			ctx.set_visuals(if dark {
				egui::Visuals::dark()
			} else {
				egui::Visuals::light()
			});
			let mut images = crate::avatars::Avatars::default();
			let mut opening = None;
			let mut profile = None;
			let mut channel = None;
			let mut mask = 0;
			let mut render = |mask: &mut u32, events| {
				let mut output = ctx.run_ui(
					egui::RawInput {
						screen_rect: Some(egui::Rect::from_min_size(
							egui::Pos2::ZERO,
							egui::vec2(width, 500.0),
						)),
						events,
						..Default::default()
					},
					|ui| {
						let mut surface = crate::select::Surface::new(ui, "body");
						parsed.show_references(
							ui,
							&mut opening,
							&[],
							None,
							&mut profile,
							(&[], &mut channel, &[], &[]),
							(&mut images, false, mask),
							&mut surface,
						);
						surface.finish(ui);
					},
				);
				assert!(output.platform_output.commands.is_empty());
				assert!(opening.is_none() && profile.is_none() && channel.is_none());
				let requests = images.take_requests();
				if *mask == 0 {
					assert!(requests.is_empty());
				}
				let mut texts = Vec::new();
				for shape in &output.shapes {
					collect(&shape.shape, &mut texts);
				}
				output.textures_delta.clear();
				output.drop_without_applying_deltas();
				texts
			};
			render(&mut mask, vec![]);
			let texts = render(&mut mask, vec![]);
			assert_eq!(
				texts
					.iter()
					.filter(|(text, _)| text == "Reveal spoiler")
					.count(),
				1
			);
			assert!(texts.iter().any(|(text, _)| text.contains("Visible")));
			assert!(!texts.iter().any(|(text, _)| text.contains("secret")
				|| text.contains("hidden")
				|| text.contains("9001")
				|| text.contains("42")));
			render(&mut mask, vec![key(egui::Key::Tab)]);
			render(&mut mask, vec![key(egui::Key::Enter)]);
			assert_eq!(mask, 1);
			let texts = render(&mut mask, vec![]);
			assert!(texts.iter().any(|(text, _)| text.contains("secret")));
			mask = 0;
			let texts = render(&mut mask, vec![]);
			assert!(!texts.iter().any(|(text, _)| text.contains("secret")));
			let pos = texts
				.iter()
				.find(|(text, _)| text == "Reveal spoiler")
				.unwrap()
				.1
				.center();
			render(
				&mut mask,
				vec![
					egui::Event::PointerMoved(pos),
					egui::Event::PointerButton {
						pos,
						button: egui::PointerButton::Primary,
						pressed: true,
						modifiers: egui::Modifiers::NONE,
					},
				],
			);
			render(
				&mut mask,
				vec![egui::Event::PointerButton {
					pos,
					button: egui::PointerButton::Primary,
					pressed: false,
					modifiers: egui::Modifiers::NONE,
				}],
			);
			assert_eq!(mask, 1);
		}
	}

	#[test]
	fn selecting_across_a_concealed_region_cannot_copy_its_text() {
		let ctx = egui::Context::default();
		let parsed = Formatted::parse("A ||private spoiler|| Z");
		let mut images = crate::avatars::Avatars::default();
		let mut mask = 0;
		let mut clock = 0.0;
		let mut run = |events| {
			clock += 1.0;
			ctx.run_ui(
				egui::RawInput {
					screen_rect: Some(egui::Rect::from_min_size(
						egui::Pos2::ZERO,
						egui::vec2(700.0, 200.0),
					)),
					events,
					time: Some(clock),
					..Default::default()
				},
				|ui| {
					let mut surface = crate::select::Surface::new(ui, "body");
					parsed.show_references(
						ui,
						&mut None,
						&[],
						None,
						&mut None,
						(&[], &mut None, &[], &[]),
						(&mut images, false, &mut mask),
						&mut surface,
					);
					surface.finish(ui);
				},
			)
		};
		run(vec![]).drop_without_applying_deltas();
		let output = run(vec![]);
		let text_rect = |prefix: &str| {
			output
				.shapes
				.iter()
				.find_map(|shape| {
					if let egui::Shape::Text(text) = &shape.shape
						&& text.galley.text().starts_with(prefix)
					{
						Some(egui::Rect::from_min_size(text.pos, text.galley.size()))
					} else {
						None
					}
				})
				.expect("visible selectable text")
		};
		let start = text_rect("A ").left_top() + egui::vec2(0.0, 5.0);
		let end = text_rect(" Z").right_top() + egui::vec2(0.0, 5.0);
		output.drop_without_applying_deltas();
		for events in [
			vec![
				egui::Event::PointerMoved(start),
				egui::Event::PointerButton {
					pos: start,
					button: egui::PointerButton::Primary,
					pressed: true,
					modifiers: egui::Modifiers::NONE,
				},
			],
			vec![egui::Event::PointerMoved(end)],
			vec![egui::Event::PointerButton {
				pos: end,
				button: egui::PointerButton::Primary,
				pressed: false,
				modifiers: egui::Modifiers::NONE,
			}],
		] {
			run(events).drop_without_applying_deltas();
		}
		let output = run(vec![egui::Event::Copy]);
		let copied = output
			.platform_output
			.commands
			.iter()
			.find_map(|command| match command {
				egui::OutputCommand::CopyText(text) => Some(text.as_str()),
				_ => None,
			})
			.expect("selected visible text");
		assert!(
			copied.contains('A') && copied.contains('Z') && !copied.contains("private spoiler")
		);
		output.drop_without_applying_deltas();
		assert_eq!(mask, 0);
	}

	#[test]
	fn channel_references_keep_literals_bounded_and_offer_missing_channels() {
		let parsed = Formatted::parse(
			"**<#42>** `<#43>` \\<#44> &lt;#45&gt; [<#46>](https://example.com) <#0>\n\n```\n<#47>\n```",
		);
		assert_eq!(
			parsed
				.spans
				.iter()
				.filter_map(|(_, style)| style.channel)
				.collect::<Vec<_>>(),
			vec![Id(42)]
		);
		assert!(
			parsed
				.spans
				.iter()
				.any(|(_, style)| style.channel == Some(Id(42)) && style.strong)
		);
		for source in ["&lt;#42&gt; <#42>", "&#60;#42&#62; <#42>", "\\<#42> <#42>"] {
			let parsed = Formatted::parse(source);
			assert_eq!(
				parsed
					.spans
					.iter()
					.take_while(|(_, style)| style.channel.is_none())
					.map(|(text, _)| text.as_str())
					.collect::<String>(),
				"<#42> "
			);
			assert_eq!(
				parsed
					.spans
					.iter()
					.filter_map(|(_, style)| style.channel)
					.collect::<Vec<_>>(),
				vec![Id(42)]
			);
		}
		let bounded = Formatted::parse(&"<#42> <@43> ".repeat(60));
		assert_eq!(bounded.mention_count, model::MAX_MENTIONS);
		assert!(bounded.limited);
		assert_eq!(
			bounded
				.spans
				.iter()
				.filter(|(_, style)| style.mention.is_some() || style.channel.is_some())
				.count(),
			model::MAX_MENTIONS
		);
		let channels: Vec<_> = [
			(1, 2, Some(Id(9))),
			(2, 4, Some(Id(9))),
			(3, 1, None),
			(4, 0, Some(Id(9))),
		]
		.into_iter()
		.map(|(id, kind, guild)| model::Channel {
			id: Id(id),
			kind,
			guild,
			name: "synthetic-text".into(),
			last_message: None,
			parent_id: None,
			position: 0,
			recipients: vec![],
			member_list_id: None,
			message_count: None,
			icon: None,
		})
		.collect();
		for id in [1, 2, 3, 4, 5] {
			let parsed = Formatted::parse(&format!("<#{}>", id));
			let ctx = egui::Context::default();
			let mut opening = None;
			let mut profile = None;
			let mut channel = None;
			let mut revealed = u32::MAX;
			for key in [egui::Key::Tab, egui::Key::Enter] {
				let mut output = ctx.run_ui(
					egui::RawInput {
						events: vec![egui::Event::Key {
							key,
							physical_key: None,
							pressed: true,
							repeat: false,
							modifiers: egui::Modifiers::NONE,
						}],
						..Default::default()
					},
					|ui| {
						let mut surface = crate::select::Surface::new(ui, "body");
						parsed.show_references(
							ui,
							&mut opening,
							&[],
							None,
							&mut profile,
							(&channels, &mut channel, &[], &[]),
							(&mut crate::avatars::Avatars::default(), true, &mut revealed),
							&mut surface,
						);
						surface.finish(ui);
					},
				);
				assert!(output.platform_output.commands.is_empty());
				output.textures_delta.clear();
			}
			assert_eq!(channel, matches!(id, 4 | 5).then_some(Id(id)));
			assert!(opening.is_none() && profile.is_none());
		}
	}
	#[test]
	fn selecting_across_images_copies_unicode_and_custom_markup() {
		let ctx = egui::Context::default();
		crate::emoji::install(&ctx).unwrap();
		let source = "A 👩🏽‍💻 ❤️ <:serein_wave:9001> Z";
		let parsed = Formatted::parse(source);
		let mut avatars = crate::avatars::Avatars::default();
		let mut clock = 0.0;
		let mut run = |events| {
			clock += 1.0;
			ctx.run_ui(
				egui::RawInput {
					screen_rect: Some(egui::Rect::from_min_size(
						egui::Pos2::ZERO,
						egui::vec2(800.0, 200.0),
					)),
					events,
					time: Some(clock),
					..Default::default()
				},
				|ui| {
					parsed.show_with_images(
						ui,
						&mut None,
						&[],
						None,
						&mut None,
						(&mut avatars, true, &[]),
					)
				},
			)
		};
		let mut output = run(vec![]);
		let mut start = egui::Pos2::ZERO;
		let mut end = egui::Pos2::ZERO;
		let mut custom_rect = egui::Rect::NOTHING;
		for shape in &output.shapes {
			// Text and artwork share one galley so rows with emoji grow before text is placed.
			if let egui::Shape::Text(text) = &shape.shape
				&& text.galley.text() == source
			{
				let row = &text.galley.rows[0];
				start = text.pos + egui::vec2(0.0, 5.0);
				end = text.pos + egui::vec2(row.size.x, 5.0);
				let markup = row
					.glyphs
					.iter()
					.find(|glyph| glyph.chr == '<')
					.expect("custom emoji glyph");
				custom_rect = egui::Rect::from_min_size(
					text.pos + egui::vec2(markup.pos.x, 0.0),
					egui::vec2(markup.advance_width, row.size.y),
				);
			}
		}
		output.textures_delta.clear();
		assert!(end.x > start.x && custom_rect.width() > 0.0);
		for events in [
			vec![
				egui::Event::PointerMoved(start),
				egui::Event::PointerButton {
					pos: start,
					button: egui::PointerButton::Primary,
					pressed: true,
					modifiers: egui::Modifiers::NONE,
				},
			],
			vec![egui::Event::PointerMoved(end)],
			vec![egui::Event::PointerButton {
				pos: end,
				button: egui::PointerButton::Primary,
				pressed: false,
				modifiers: egui::Modifiers::NONE,
			}],
		] {
			run(events).drop_without_applying_deltas();
		}
		let mut output = run(vec![egui::Event::Copy]);
		output.textures_delta.clear();
		let copied = output
			.platform_output
			.commands
			.iter()
			.find_map(|command| match command {
				egui::OutputCommand::CopyText(text) => Some(text.as_str()),
				_ => None,
			});
		assert_eq!(copied.map(str::trim_end), Some(source));
		for fraction in [0.25, 0.75] {
			let start = custom_rect.left_top() + egui::vec2(custom_rect.width() * fraction, 5.0);
			for events in [
				vec![
					egui::Event::PointerMoved(start),
					egui::Event::PointerButton {
						pos: start,
						button: egui::PointerButton::Primary,
						pressed: true,
						modifiers: egui::Modifiers::NONE,
					},
				],
				vec![egui::Event::PointerMoved(end)],
				vec![egui::Event::PointerButton {
					pos: end,
					button: egui::PointerButton::Primary,
					pressed: false,
					modifiers: egui::Modifiers::NONE,
				}],
			] {
				run(events).drop_without_applying_deltas();
			}
			let mut output = run(vec![egui::Event::Copy]);
			output.textures_delta.clear();
			let copied = output
				.platform_output
				.commands
				.iter()
				.find_map(|command| match command {
					egui::OutputCommand::CopyText(text) => Some(text.as_str()),
					_ => None,
				});
			assert!(
				matches!(
					copied.map(str::trim_end),
					Some("<:serein_wave:9001> Z" | " Z")
				),
				"partial emoji copied: {copied:?}"
			);
		}
	}
	#[test]
	fn emoji_click_shows_default_known_or_unknown_source_and_escape_closes() {
		fn text_shapes<'a>(shape: &'a egui::Shape, texts: &mut Vec<&'a egui::epaint::TextShape>) {
			match shape {
				egui::Shape::Text(text) => texts.push(text),
				egui::Shape::Vec(shapes) => {
					for shape in shapes {
						text_shapes(shape, texts);
					}
				}
				_ => {}
			}
		}
		for light in [false, true] {
			for (source, title, description) in [
				(
					"\u{1f9c2}",
					":salt:",
					"A default emoji. You can use this emoji everywhere on Discord.",
				),
				(
					"<:old_name:9001>",
					":serein_wave:",
					"From Emoji source server",
				),
				(
					"<:unknown:999999>",
					":unknown:",
					"Source server unavailable in this session.",
				),
			] {
				let ctx = egui::Context::default();
				if light {
					ctx.set_visuals(egui::Visuals::light());
				}
				crate::emoji::install(&ctx).unwrap();
				let mut state = test_support::demo_state();
				for guild in &mut state.guilds {
					guild.name = "Emoji source server".into();
				}
				let parsed = Formatted::parse(source);
				let mut images = crate::avatars::Avatars::default();
				let mut clock = 0.0;
				let mut frame = |events| {
					clock += 0.02;
					ctx.run_ui(
						egui::RawInput {
							screen_rect: Some(egui::Rect::from_min_size(
								egui::Pos2::ZERO,
								egui::vec2(360.0, 300.0),
							)),
							events,
							time: Some(clock),
							..Default::default()
						},
						|ui| {
							parsed.show_with_images(
								ui,
								&mut None,
								&[],
								None,
								&mut None,
								(&mut images, true, &state.guilds),
							)
						},
					)
				};
				let mut output = frame(vec![]);
				let mut texts = vec![];
				for shape in &output.shapes {
					text_shapes(&shape.shape, &mut texts);
				}
				let text = texts
					.iter()
					.find(|text| text.galley.text() == source)
					.unwrap();
				let row = &text.galley.rows[0];
				let glyph = &row.glyphs[0];
				let point = text.pos
					+ egui::vec2(glyph.pos.x + glyph.advance_width / 2.0, row.size.y / 2.0);
				output.textures_delta.clear();
				frame(vec![egui::Event::PointerMoved(point)]).drop_without_applying_deltas();
				for pressed in [true, false] {
					frame(vec![egui::Event::PointerButton {
						pos: point,
						button: egui::PointerButton::Primary,
						pressed,
						modifiers: egui::Modifiers::NONE,
					}])
					.drop_without_applying_deltas();
				}
				frame(vec![]).drop_without_applying_deltas();
				let output = frame(vec![]);
				let mut texts = vec![];
				for shape in &output.shapes {
					text_shapes(&shape.shape, &mut texts);
				}
				assert!(
					texts.iter().any(|text| text.galley.text() == title),
					"missing title {title}"
				);
				assert!(
					texts.iter().any(|text| text.galley.text() == description),
					"missing description {description}"
				);
				for text in texts {
					let rect = text.galley.rect.translate(text.pos.to_vec2());
					assert!(
						rect.right() <= 360.5 && rect.bottom() <= 300.5,
						"clipped emoji details: {rect:?}"
					);
				}
				output.drop_without_applying_deltas();
				assert!(egui::Popup::is_any_open(&ctx));
				frame(vec![egui::Event::Key {
					key: egui::Key::Escape,
					physical_key: None,
					pressed: true,
					repeat: false,
					modifiers: egui::Modifiers::NONE,
				}])
				.drop_without_applying_deltas();
				assert!(!egui::Popup::is_any_open(&ctx));
			}
		}
	}
	#[test]
	fn loading_emoji_reserve_the_same_message_space_without_font_fallback() {
		let ctx = egui::Context::default();
		let parsed = Formatted::parse("😀👩🏽‍💻❤️🇨🇿");
		let mut cold_size = None;
		for ready in [false, true] {
			if ready {
				crate::emoji::install(&ctx).unwrap();
			}
			let output = ctx.run_ui(Default::default(), |ui| {
				ui.set_max_width(65.0);
				parsed.show(ui, &mut None);
				if let Some(size) = cold_size {
					assert_eq!(ui.min_size(), size);
				} else {
					cold_size = Some(ui.min_size());
				}
			});
			let mut images = 0;
			for shape in &output.shapes {
				match &shape.shape {
					egui::Shape::Text(text) => {
						assert!(
							text.galley
								.rows
								.iter()
								.all(|row| row.visuals.mesh.is_empty())
						);
					}
					egui::Shape::Rect(rect) if rect.brush.is_some() => images += 1,
					_ => {}
				}
			}
			assert_eq!(images, if ready { 4 } else { 0 });
			output.drop_without_applying_deltas();
		}
	}

	#[test]
	fn emoji_render_as_whole_images_but_code_and_source_stay_literal() {
		let ctx = egui::Context::default();
		crate::emoji::install(&ctx).unwrap();
		let source = "👩🏽‍💻 `😀` 🇨🇿";
		let parsed = Formatted::parse(source);
		assert!(
			parsed
				.spans
				.iter()
				.any(|(text, style)| style.code && text == "😀")
		);
		let output = ctx.run_ui(Default::default(), |ui| {
			parsed.show(ui, &mut None);
		});
		let images = output
			.shapes
			.iter()
			.filter(|shape| {
				matches!(
					&shape.shape, egui::Shape::Rect(rect) if rect.brush.is_some()
				)
			})
			.count();
		output.drop_without_applying_deltas();
		assert_eq!(images, 2, "one image per complete grapheme, none in code");
	}
	#[test]
	fn fenced_blocks_record_language_and_discord_fence_shapes() {
		let parsed = Formatted::parse("intro\n```js\nconst a = 1;\n```\nafter");
		assert_eq!(parsed.blocks.len(), 1);
		assert_eq!(
			parsed.blocks[0].language,
			Some(crate::highlight::Language::JavaScript)
		);
		assert_eq!(parsed.blocks[0].code, "const a = 1;");
		assert!(!parsed.blocks[0].segments.is_empty());
		let block_spans: Vec<&str> = parsed
			.spans
			.iter()
			.filter(|(_, style)| style.block == Some(0))
			.map(|(text, _)| text.as_str())
			.collect();
		assert_eq!(block_spans.concat(), "const a = 1;\n");
		assert!(
			parsed
				.spans
				.iter()
				.any(|(t, s)| t == "after" && s.block.is_none() && !s.code)
		);

		// Closing fence at the end of the last content line, then text on the fence line.
		let parsed = Formatted::parse("```py\nprint(1)``` trailing");
		assert_eq!(parsed.blocks.len(), 1);
		assert_eq!(parsed.blocks[0].code, "print(1)");
		assert_eq!(
			parsed.blocks[0].language,
			Some(crate::highlight::Language::Python)
		);
		assert!(
			parsed
				.spans
				.iter()
				.any(|(t, s)| t.contains("trailing") && s.block.is_none())
		);

		// One-line triple backticks are a block without a language; unknown tags keep their name.
		let parsed = Formatted::parse("```echo hi``` and `inline`");
		assert_eq!(parsed.blocks.len(), 1);
		assert_eq!(parsed.blocks[0].code, "echo hi");
		assert!(parsed.blocks[0].language.is_none() && parsed.blocks[0].tag.is_empty());
		assert!(
			parsed
				.spans
				.iter()
				.any(|(t, s)| t == "inline" && s.code && s.block.is_none())
		);
		let parsed = Formatted::parse("```elixir\nIO.puts 1\n```");
		assert_eq!(parsed.blocks[0].tag, "elixir");
		assert!(parsed.blocks[0].language.is_none());
		assert_eq!(parsed.blocks[0].segments, Vec::new());

		// Discord has no indented code blocks and bounds the block count.
		let parsed = Formatted::parse("text\n\n    not code");
		assert!(parsed.blocks.is_empty());
		assert!(parsed.spans.iter().all(|(_, style)| !style.code));
		let many = "```\nx\n```\n".repeat(MAX_BLOCKS + 4);
		let parsed = Formatted::parse(&many);
		assert!(parsed.blocks.len() <= MAX_BLOCKS);
	}
	#[test]
	fn code_blocks_render_a_framed_widget_with_copy_control() {
		fn walk<'a>(shape: &'a egui::Shape, out: &mut Vec<&'a egui::Shape>) {
			match shape {
				egui::Shape::Vec(shapes) => shapes.iter().for_each(|s| walk(s, out)),
				other => out.push(other),
			}
		}
		let ctx = egui::Context::default();
		crate::icons::install(&ctx);
		let parsed = Formatted::parse("before\n```rust\nfn main() {}\n```\nafter");
		let frame = |events| {
			ctx.run_ui(
				egui::RawInput {
					screen_rect: Some(egui::Rect::from_min_size(
						egui::Pos2::ZERO,
						egui::vec2(420.0, 300.0),
					)),
					events,
					..Default::default()
				},
				|ui| parsed.show(ui, &mut None),
			)
		};
		let mut button = None;
		for _ in 0..2 {
			let output = frame(vec![]);
			let code_bg = ctx.global_style().visuals.code_bg_color;
			let mut shapes = Vec::new();
			output
				.shapes
				.iter()
				.for_each(|s| walk(&s.shape, &mut shapes));
			let texts: Vec<(&str, egui::Pos2, egui::Vec2, egui::FontFamily)> = shapes
				.iter()
				.filter_map(|shape| match shape {
					egui::Shape::Text(text) => Some((
						text.galley.text(),
						text.pos,
						text.galley.size(),
						text.galley.job.sections[0].format.font_id.family.clone(),
					)),
					_ => None,
				})
				.collect();
			let code = texts
				.iter()
				.find(|(text, ..)| *text == "fn main() {}")
				.expect("code text");
			let before = texts
				.iter()
				.find(|(text, ..)| *text == "before")
				.expect("paragraph before code block");
			let after = texts
				.iter()
				.find(|(text, ..)| *text == "after")
				.expect("paragraph after code block");
			assert_eq!(code.3, egui::FontFamily::Monospace);
			assert!(
				texts.iter().any(|(text, ..)| *text == "Rust"),
				"language header"
			);
			assert!(
				texts.iter().any(|(text, ..)| *text == "before"),
				"paragraph newline before the block is dropped: {texts:?}"
			);
			assert!(texts.iter().any(|(text, ..)| *text == "after"));
			let bg = shapes
				.iter()
				.find_map(|shape| match shape {
					egui::Shape::Rect(rect) if rect.fill == code_bg => Some(rect.rect),
					_ => None,
				})
				.expect("framed background");
			assert!(bg.contains_rect(egui::Rect::from_min_size(code.1, code.2)));
			assert!(
				before.1.y + before.2.y <= bg.top(),
				"paragraph before the block overlaps its frame: before={before:?}, block={bg:?}"
			);
			assert!(
				after.1.y >= bg.bottom(),
				"paragraph after the block overlaps its frame: after={after:?}, block={bg:?}"
			);
			let header = texts
				.iter()
				.find(|(text, ..)| *text == "Rust")
				.expect("header");
			button = Some(egui::pos2(
				bg.right() - 10.0 - 12.0,
				header.1.y + header.2.y / 2.0,
			));
			output.drop_without_applying_deltas();
		}
		let pos = button.expect("copy control");
		let output = frame(vec![
			egui::Event::PointerMoved(pos),
			egui::Event::PointerButton {
				pos,
				button: egui::PointerButton::Primary,
				pressed: true,
				modifiers: egui::Modifiers::NONE,
			},
			egui::Event::PointerButton {
				pos,
				button: egui::PointerButton::Primary,
				pressed: false,
				modifiers: egui::Modifiers::NONE,
			},
		]);
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
		assert_eq!(copied, vec!["fn main() {}".to_owned()]);
	}
	#[test]
	fn dragging_out_of_a_code_block_leaves_the_text_above_it_unselected() {
		let ctx = egui::Context::default();
		crate::icons::install(&ctx);
		let parsed = Formatted::parse("before the block\n```\nalpha\nbravo\n```\nafter the block");
		let frame = |events| {
			ctx.run_ui(
				egui::RawInput {
					screen_rect: Some(egui::Rect::from_min_size(
						egui::Pos2::ZERO,
						egui::vec2(420.0, 300.0),
					)),
					events,
					..Default::default()
				},
				|ui| parsed.show(ui, &mut None),
			)
		};
		let press = |pos, pressed| {
			vec![
				egui::Event::PointerMoved(pos),
				egui::Event::PointerButton {
					pos,
					button: egui::PointerButton::Primary,
					pressed,
					modifiers: egui::Modifiers::NONE,
				},
			]
		};
		// Locate the painted galleys, so the drag uses real glyph positions.
		let mut code = None;
		let mut after = None;
		for _ in 0..2 {
			let output = frame(Vec::new());
			let mut shapes = Vec::new();
			fn walk<'a>(shape: &'a egui::Shape, out: &mut Vec<&'a egui::Shape>) {
				match shape {
					egui::Shape::Vec(shapes) => shapes.iter().for_each(|s| walk(s, out)),
					other => out.push(other),
				}
			}
			output
				.shapes
				.iter()
				.for_each(|s| walk(&s.shape, &mut shapes));
			let galley = |wanted: &str| {
				shapes.iter().find_map(|shape| match shape {
					egui::Shape::Text(text) if text.galley.text() == wanted => {
						Some(egui::Rect::from_min_size(text.pos, text.galley.size()))
					}
					_ => None,
				})
			};
			code = galley("alpha\nbravo");
			after = galley("after the block");
			output.drop_without_applying_deltas();
		}
		let code = code.expect("painted code galley");
		let after = after.expect("painted trailing paragraph");
		// Out of the block's last line and into the paragraph under it: the paragraph above
		// the block is registered with the rest of the body, and must stay unselected.
		let from = egui::pos2(code.left() + 1.0, code.bottom() - 2.0);
		let to = egui::pos2(after.right() - 1.0, after.center().y);
		let mut copied = String::new();
		for events in [
			press(from, true),
			vec![egui::Event::PointerMoved(to)],
			press(to, false),
			vec![egui::Event::Copy],
		] {
			let output = frame(events);
			if let Some(text) =
				output
					.platform_output
					.commands
					.iter()
					.find_map(|command| match command {
						egui::OutputCommand::CopyText(text) => Some(text.clone()),
						_ => None,
					}) {
				copied = text;
			}
			output.drop_without_applying_deltas();
		}
		assert_eq!(copied, "bravo\n\nafter the block");
	}
	#[test]
	fn an_unlabelled_code_block_floats_a_copy_control_above_its_text() {
		let ctx = egui::Context::default();
		crate::icons::install(&ctx);
		let parsed = Formatted::parse("```\nfirst line of code\nsecond line\nthird line\n```");
		let frame = |events| {
			ctx.run_ui(
				egui::RawInput {
					screen_rect: Some(egui::Rect::from_min_size(
						egui::Pos2::ZERO,
						egui::vec2(420.0, 300.0),
					)),
					events,
					..Default::default()
				},
				|ui| parsed.show(ui, &mut None),
			)
		};
		let mut block = None;
		for _ in 0..2 {
			let output = frame(Vec::new());
			let code_bg = ctx.global_style().visuals.code_bg_color;
			block = output.shapes.iter().find_map(|shape| match &shape.shape {
				egui::Shape::Rect(rect) if rect.fill == code_bg => Some(rect.rect),
				_ => None,
			});
			output.drop_without_applying_deltas();
		}
		// No language header, so the control floats in the block's top-right corner and
		// only while the pointer is inside it.
		let block = block.expect("framed background");
		let pos = egui::pos2(block.right() - 17.0, block.top() + 17.0);
		let hover = frame(vec![egui::Event::PointerMoved(pos)]);
		let code = hover
			.shapes
			.iter()
			.position(|shape| matches!(&shape.shape, egui::Shape::Text(text) if text.galley.text().starts_with("first line")))
			.expect("painted code galley");
		let control = hover
			.shapes
			.iter()
			.position(|shape| {
				let bounds = shape.shape.visual_bounding_rect();
				bounds.contains(pos) && bounds.width() < 40.0
			})
			.expect("floating control");
		hover.drop_without_applying_deltas();
		assert!(
			code < control,
			"the block's deferred text must paint under the control, not over it"
		);
		let output = frame(vec![
			egui::Event::PointerMoved(pos),
			egui::Event::PointerButton {
				pos,
				button: egui::PointerButton::Primary,
				pressed: true,
				modifiers: egui::Modifiers::NONE,
			},
			egui::Event::PointerButton {
				pos,
				button: egui::PointerButton::Primary,
				pressed: false,
				modifiers: egui::Modifiers::NONE,
			},
		]);
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
		assert_eq!(
			copied,
			vec!["first line of code\nsecond line\nthird line".to_owned()]
		);
	}
	#[test]
	fn mass_mentions_render_as_pills_only_for_exact_plain_tokens() {
		let parsed = Formatted::parse("@everyone @here `@everyone` @everyone_else \\@here");
		assert_eq!(
			parsed
				.spans
				.iter()
				.filter(|(_, style)| style.mass_mention)
				.map(|(text, _)| text.as_str())
				.collect::<Vec<_>>(),
			["@everyone", "@here"]
		);
		let ctx = egui::Context::default();
		let output = ctx.run_ui(Default::default(), |ui| parsed.show(ui, &mut None));
		let colors = crate::design::palette_for(&ctx);
		let highlighted = output
			.shapes
			.iter()
			.filter_map(|shape| match &shape.shape {
				egui::Shape::Text(text) => Some(&text.galley.job.sections),
				_ => None,
			})
			.flatten()
			.filter(|section| {
				section.format.background == colors.mention_bg
					&& section.format.color == colors.mention_text
			})
			.count();
		assert_eq!(highlighted, 2);
		output.drop_without_applying_deltas();
	}
	#[test]
	fn timestamps_render_the_formatted_instant_not_the_raw_token() {
		let parsed = Formatted::parse(
			"<t:1700000000:R> <t:1700000000> `<t:1700000000:t>` \\<t:1:t> <t:1:z> <t:abc:t>",
		);
		let stamps: Vec<_> = parsed
			.spans
			.iter()
			.filter_map(|(text, style)| style.timestamp.map(|stamp| (text.as_str(), stamp)))
			.collect();
		assert_eq!(
			stamps,
			vec![
				("<t:1700000000:R>", (1_700_000_000, b'R')),
				("<t:1700000000>", (1_700_000_000, b'f')),
			]
		);
		let ctx = egui::Context::default();
		let mut output = ctx.run_ui(Default::default(), |ui| {
			ui.set_width(400.0);
			parsed.show(ui, &mut None);
		});
		output.textures_delta.clear();
		output.drop_without_applying_deltas();
		let mut job = LayoutJob::default();
		ctx.run_ui(Default::default(), |ui| {
			parsed.append_inline_preview(&mut job, ui, &[], None, &[], &[]);
		})
		.drop_without_applying_deltas();
		assert!(job.text.contains("ago"), "relative style: {}", job.text);
		assert!(
			job.text.contains("November 14, 2023"),
			"default style: {}",
			job.text
		);
		// Code spans, escapes and unknown styles keep the literal source.
		assert_eq!(job.text.matches("<t:").count(), 4, "{}", job.text);
	}
	#[test]
	fn mention_highlights_include_unknown_users_in_both_themes() {
		let ctx = egui::Context::default();
		let users = vec![model::User {
			id: Id(42),
			name: "Synthetic Robin".into(),
			avatar: None,
			webhook: false,
			kind: Default::default(),
			discriminator: 0,
			primary_guild: None,
		}];
		let parsed = Formatted::parse("<@42> <@!43> `<@44>` \\<@45>");
		for dark in [true, false] {
			ctx.set_visuals(if dark {
				egui::Visuals::dark()
			} else {
				egui::Visuals::light()
			});
			for width in [80.0, 300.0] {
				let mut output = ctx.run_ui(Default::default(), |ui| {
					ui.set_width(width);
					parsed.show_mentions(ui, &mut None, &users, &mut None);
				});
				output.textures_delta.clear();
				let colors = crate::design::colors(dark, crate::design::variant());
				let highlighted: Vec<_> = output
					.shapes
					.iter()
					.filter_map(|shape| {
						let egui::Shape::Text(text) = &shape.shape else {
							return None;
						};
						text.galley
							.job
							.sections
							.iter()
							.any(|section| {
								section.format.background == colors.mention_bg
									&& section.format.color == colors.mention_text
							})
							.then_some(text.galley.job.text.as_str())
					})
					.collect();
				assert_eq!(highlighted, ["@Synthetic Robin", "@43"]);
				output.drop_without_applying_deltas();
			}
		}
	}

	#[test]
	fn mentions_and_emoji_share_one_row_baseline() {
		let ctx = egui::Context::default();
		let users = vec![model::User {
			id: Id(42),
			name: "rain".into(),
			avatar: None,
			webhook: false,
			kind: Default::default(),
			discriminator: 0,
			primary_guild: None,
		}];
		for source in [
			"<@42> test \u{1f610} test <@42>",
			"<@42> test <:wave:9001> test <@42>",
			"\u{1f610} <@42> #general",
		] {
			let parsed = Formatted::parse(source);
			assert!(parsed.artwork, "{source}");
			let mut profile = None;
			let mut opening = None;
			let output = ctx.run_ui(Default::default(), |ui| {
				ui.set_width(400.0);
				parsed.show_mentions(ui, &mut opening, &users, &mut profile);
			});
			fn walk(shape: &egui::Shape, rows: &mut Vec<(f32, f32, f32)>) {
				match shape {
					egui::Shape::Text(text) => {
						for placed in &text.galley.rows {
							for glyph in &placed.row.glyphs {
								rows.push((glyph.line_height, placed.row.size.y, glyph.pos.y));
							}
						}
					}
					egui::Shape::Vec(shapes) => {
						shapes.iter().for_each(|shape| walk(shape, rows));
					}
					_ => {}
				}
			}
			let mut rows: Vec<(f32, f32, f32)> = Vec::new();
			for shape in &output.shapes {
				walk(&shape.shape, &mut rows);
			}
			// Only the body font: emoji placeholders carry the artwork font, whose own
			// ascent says nothing about where the words sit.
			let body = rows
				.iter()
				.map(|(height, ..)| *height)
				.fold(f32::INFINITY, f32::min);
			rows.retain(|(height, ..)| *height == body);
			assert!(rows.len() > 4, "{source}");
			// One shared row height and baseline: words never ride above the artwork.
			let first = rows[0];
			for row in &rows {
				assert!(
					(row.1 - first.1).abs() < 0.5 && (row.2 - first.2).abs() < 0.5,
					"{source}: {row:?} against {first:?}"
				);
			}
			output.drop_without_applying_deltas();
		}
	}
	#[test]
	fn plain_messages_keep_the_body_line_height() {
		let parsed = Formatted::parse("plain <@42> words");
		assert!(!parsed.artwork);
	}
	#[test]
	fn user_mentions_preserve_literals_and_open_native_profiles() {
		let parsed = Formatted::parse(
			"Hello **<@42>** <@!42> `<@43>` \\<@44> &lt;@45&gt; [<@46>](https://example.com) <@&47>",
		);
		assert_eq!(
			parsed
				.spans
				.iter()
				.filter_map(|(_, style)| style.mention)
				.collect::<Vec<_>>(),
			vec![Id(42), Id(42)]
		);
		assert!(
			parsed
				.spans
				.iter()
				.any(|(_, style)| style.mention == Some(Id(42)) && style.strong)
		);
		let parsed = Formatted::parse("<@42>");
		let ctx = egui::Context::default();
		let mut profile = None;
		let mut opening = None;
		let users = vec![model::User {
			id: Id(42),
			name: "Synthetic Robin".into(),
			avatar: None,
			webhook: false,
			kind: Default::default(),
			discriminator: 0,
			primary_guild: None,
		}];
		for key in [egui::Key::Tab, egui::Key::Enter] {
			let mut output = ctx.run_ui(
				egui::RawInput {
					events: vec![egui::Event::Key {
						key,
						physical_key: None,
						pressed: true,
						repeat: false,
						modifiers: egui::Modifiers::NONE,
					}],
					..Default::default()
				},
				|ui| parsed.show_mentions(ui, &mut opening, &users, &mut profile),
			);
			assert!(output.platform_output.commands.is_empty());
			output.textures_delta.clear();
		}
		assert_eq!(profile.unwrap().name, "Synthetic Robin");
		assert!(opening.is_none());
	}
	#[test]
	fn inline_links_preserve_markdown_and_activate_their_own_destinations() {
		let source = "Before [**Markdown** *label*](https://example.com/masked) then (https://example.org/a_(b)). `https://code.test` [https://label.test](javascript:bad)";
		let parsed = Formatted::parse(source);
		assert_eq!(
			parsed.links,
			["https://example.com/masked", "https://example.org/a_(b)"]
		);
		assert!(
			parsed
				.spans
				.iter()
				.any(|(s, f)| s == "Markdown" && f.strong && f.link == Some(0))
		);
		assert!(
			parsed
				.spans
				.iter()
				.any(|(s, f)| s == "label" && f.italic && f.link == Some(0))
		);
		assert!(
			parsed
				.spans
				.iter()
				.any(|(s, f)| s == "https://example.org/a_(b)" && f.link == Some(1))
		);
		let repeated = Formatted::parse("nothttps://example.org https://example.org");
		assert_eq!(repeated.spans[0].0, "nothttps://example.org ");
		assert!(repeated.spans[0].1.link.is_none());

		let ctx = egui::Context::default();
		let mut opening = None;
		let mut render = |events| {
			let mut output = ctx.run_ui(
				egui::RawInput {
					screen_rect: Some(egui::Rect::from_min_size(
						egui::Pos2::ZERO,
						egui::vec2(220.0, 500.0),
					)),
					events,
					..Default::default()
				},
				|ui| parsed.show(ui, &mut opening),
			);
			assert!(
				output.platform_output.commands.is_empty(),
				"Links must use confirmation, never open while rendering or on first activation"
			);
			output.textures_delta.clear();
			opening.take()
		};
		assert!(render(vec![]).is_none());
		// Native links participate in keyboard focus; each targets its own normalized URL.
		for target in &parsed.links {
			assert!(
				render(vec![egui::Event::Key {
					key: egui::Key::Tab,
					physical_key: None,
					pressed: true,
					repeat: false,
					modifiers: egui::Modifiers::NONE,
				}])
				.is_none()
			);
			assert_eq!(
				render(vec![egui::Event::Key {
					key: egui::Key::Enter,
					physical_key: None,
					pressed: true,
					repeat: false,
					modifiers: egui::Modifiers::NONE,
				}]),
				Some(target.clone())
			);
			render(vec![egui::Event::Key {
				key: egui::Key::Enter,
				physical_key: None,
				pressed: false,
				repeat: false,
				modifiers: egui::Modifiers::NONE,
			}]);
		}
	}
	#[test]
	fn bounded_formatting_and_inert_external_content() {
		let parsed = Formatted::parse(
			"**strong** *em* ~~gone~~ `code`\n> quote\n\n[site](https://example.com/a) ![alt](https://example.com/image) <script>inert</script>",
		);
		assert!(parsed.spans.iter().any(|(s, f)| s == "strong" && f.strong));
		assert!(parsed.spans.iter().any(|(s, f)| s == "em" && f.italic));
		assert!(parsed.spans.iter().any(|(s, f)| s == "gone" && f.strike));
		assert!(parsed.spans.iter().any(|(s, f)| s == "code" && f.code));
		assert_eq!(parsed.links, ["https://example.com/a"]);
		assert!(parsed.spans.iter().any(|(s, _)| s.contains("<script>")));
		for unsafe_url in [
			"javascript:alert(1)",
			"file:///tmp/test",
			"data:text/html,x",
			"https://owner:secret@example.com",
			"https://example.com\n",
			"https:\\example.com",
		] {
			assert!(external_url(unsafe_url).is_none());
		}
		assert_eq!(
			external_url("https://例え.jp"),
			Some("https://xn--r8jz45g.jp/".into())
		);
		let deep = Formatted::parse(&format!("{}text", "> ".repeat(100)));
		assert!(deep.limited && deep.links.is_empty());
		let huge = Formatted::parse(&"日本語".repeat(10_000));
		assert!(huge.limited && huge.bytes() < 16 * 1024);
		assert!(Formatted::parse("||**concealed**||").spoilers);
		assert_eq!(
			Formatted::parse("https://example.com/a, `https://example.com/private`").links,
			["https://example.com/a"]
		);
		let mut cache = FormatCache::default();
		for id in 0..500 {
			cache.get(Id(id), &format!("{id} {}", "日本語".repeat(2000)));
		}
		assert!(cache.entries.len() <= 512 && cache.bytes <= 1024 * 1024);
		assert!(cache.get(Id(499), "||changed||").spoilers);
		cache.retain(|_| false);
		assert!(cache.entries.is_empty() && cache.bytes == 0);
		for id in 0..500 {
			cache.get(Id(id), "short message");
		}
		assert_eq!(cache.entries.len(), 500);
		assert!(cache.bytes <= 1024 * 1024);
	}
}
