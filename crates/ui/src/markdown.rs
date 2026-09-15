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

#[derive(Clone, Copy, Default)]
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
	mass_mention: bool,
	channel: Option<Id>,
	no_autolink: bool,
	spoiler: Option<u8>,
	/// Fenced code block index; the block widget replaces these spans when shown.
	block: Option<u8>,
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
					for _ in 0..blank {
						output.push("\n", Style::default());
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
							if style.quote && output.line_start() {
								output.push("│ ", style);
							}
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
						Tag::CodeBlock(CodeBlockKind::Indented) => {
							if style.quote && output.line_start() {
								output.push("│ ", style);
							}
						}
						Tag::Paragraph => {
							if style.quote && output.line_start() {
								output.push("│ ", style);
							}
						}
						Tag::BlockQuote(_) => {
							style.quote = true;
							if input[range.start..].starts_with(">>>") {
								quote_all = true;
							}
						}
						Tag::List(start) => lists.push(start),
						Tag::Item => {
							if style.quote && output.line_start() {
								output.push("│ ", style);
							}
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
						let next = input[range.end..].trim_start_matches(' ');
						if next.starts_with('>') {
							output.push("\n│ ", style);
						} else {
							quote_lazy = true;
							output.push("\n", style);
						}
					} else if style.quote {
						output.push("\n│ ", style);
					} else {
						output.push("\n", style);
					}
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
		output
	}
	fn limited_literal(input: &str, concealed: bool) -> Self {
		Self {
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
		}
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
			let reference = &text[start..];
			let (id, len, is_channel, mass_mention) =
				if let Some(len) = model::mass_mention_prefix(reference) {
					(None, len, false, true)
				} else {
					let is_channel = reference.starts_with("<#");
					let Some((id, len)) = (if is_channel {
						model::channel_mention_prefix(reference)
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
			if source[..raw_start]
				.bytes()
				.rev()
				.take_while(|b| *b == b'\\')
				.count() % 2 == 1
			{
				continue;
			}
			self.push_autolinks(&text[consumed..start], style);
			self.push(
				token,
				Style {
					mention: id.filter(|_| !is_channel),
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
			profile,
			(&mut crate::avatars::Avatars::default(), true, &[]),
		);
	}
	pub fn show_with_images(
		&self,
		ui: &mut egui::Ui,
		opening: &mut Option<String>,
		users: &[model::User],
		profile: &mut Option<model::User>,
		media: (&mut crate::avatars::Avatars, bool, &[model::Guild]),
	) {
		let (images, demo, guilds) = media;
		let mut revealed = u32::MAX;
		self.show_references(
			ui,
			opening,
			users,
			profile,
			(&[], &mut None, guilds),
			(images, demo, &mut revealed),
		);
	}
	pub fn show_references(
		&self,
		ui: &mut egui::Ui,
		opening: &mut Option<String>,
		users: &[model::User],
		profile: &mut Option<model::User>,
		references: (&[model::Channel], &mut Option<Id>, &[model::Guild]),
		media: (&mut crate::avatars::Avatars, bool, &mut u32),
	) {
		let (channels, channel, guilds) = references;
		let (images, demo, revealed) = media;
		ui.allocate_ui_with_layout(
			egui::vec2(ui.available_width(), 0.0),
			egui::Layout::left_to_right(egui::Align::Min).with_main_wrap(true),
			|ui| {
				ui.spacing_mut().item_spacing = egui::Vec2::ZERO;
				ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
				let mut start = 0;
				while start < self.spans.len() {
					let spoiler = self.spans[start].1.spoiler;
					if let Some(region) = spoiler
						&& *revealed & (1_u32 << region) == 0
					{
						// Hidden text never reaches labels, selection, tooltips, links,
						// mention actions, accessibility values or emoji image requests.
						let count = self.spans[start..]
							.iter()
							.take_while(|(_, style)| style.spoiler == spoiler)
							.count();
						if ui
							.push_id(("spoiler", region), |ui| ui.button("Reveal spoiler"))
							.inner
							.clicked()
						{
							*revealed |= 1_u32 << region;
						}
						start += count;
						continue;
					}
					if self.spans[start].0.is_empty() {
						start += 1;
						continue;
					}
					if let Some(id) = self.spans[start].1.channel {
						if let Some(target) = channels.iter().find(|target| {
							target.id == id && target.guild.is_some() && target.supports_text()
						}) {
							let label = format!("#{}", target.name);
							let response = ui
								.add(egui::Link::new(egui::RichText::new(&label).strong()))
								.on_hover_text("Open channel");
							response.widget_info(|| {
								egui::WidgetInfo::labeled(
									egui::WidgetType::Link,
									ui.is_enabled(),
									format!("{label}, open channel"),
								)
							});
							if response.clicked() {
								*channel = Some(id);
							}
						} else if channels.iter().all(|target| target.id != id) {
							let label = "#unknown-channel";
							let response = ui
								.add(egui::Link::new(egui::RichText::new(label).strong()))
								.on_hover_text("Load channel");
							response.widget_info(|| {
								egui::WidgetInfo::labeled(
									egui::WidgetType::Link,
									ui.is_enabled(),
									"Unknown channel, load channel",
								)
							});
							if response.clicked() {
								*channel = Some(id);
							}
						} else {
							ui.add(egui::Label::new(&self.spans[start].0).selectable(true))
								.on_hover_text(
									"Channel unavailable or unsupported in this session",
								);
						}
						start += 1;
						continue;
					}
					if let Some(id) = self.spans[start].1.mention {
						let colors = crate::design::palette(ui);
						let user = users.iter().find(|user| user.id == id);
						let label = format!(
							"@{}",
							user.map_or_else(|| id.to_string(), |u| u.name.clone())
						);
						let response = ui
							.add(egui::Link::new(
								egui::RichText::new(&label)
									.strong()
									.color(colors.mention_text)
									.background_color(colors.mention_bg),
							))
							.on_hover_text("Open user profile");
						response.widget_info(|| {
							egui::WidgetInfo::labeled(
								egui::WidgetType::Link,
								ui.is_enabled(),
								format!("{label}, user profile"),
							)
						});
						if response.clicked() {
							*profile = Some(user.cloned().unwrap_or(model::User {
								id,
								name: format!("User {id}"),
								avatar: None,
								webhook: false,
								kind: Default::default(),
								discriminator: 0,
							}));
						}
						start += 1;
						continue;
					}
					if let Some(block) = self.spans[start].1.block {
						let count = self.spans[start..]
							.iter()
							.take_while(|(_, style)| style.block == Some(block))
							.count();
						Self::show_code_block(ui, &self.blocks[usize::from(block)], block);
						start += count;
						continue;
					}
					let target = self.spans[start].1.link;
					let count =
						self.spans[start..]
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
						&& self.spans[start + count - 1].0.ends_with('\n')
					{
						let mut copy = self.spans[start..start + count].to_vec();
						let last = &mut copy[count - 1].0;
						last.truncate(last.len() - 1);
						trimmed = copy;
						&trimmed[..]
					} else {
						&self.spans[start..start + count]
					};
					if let Some(index) = target {
						let url = &self.links[index];
						let label: String = spans.iter().map(|(text, _)| text.as_str()).collect();
						let response = Self::show_emoji(spans, ui, true, images, demo, guilds)
							.on_hover_text(url);
						// Text selection in egui's Link overwrites its accessibility role.
						response.widget_info(|| {
							egui::WidgetInfo::labeled(
								egui::WidgetType::Link,
								ui.is_enabled(),
								&label,
							)
						});
						if response.clicked() {
							*opening = Some(url.clone());
						}
					} else {
						Self::show_emoji(spans, ui, false, images, demo, guilds);
					}
					start += count;
				}
			},
		);
	}
	/// Full-width framed block: optional language header with a copy control, then the
	/// highlighted, wrapped, selectable monospace text.
	fn show_code_block(ui: &mut egui::Ui, block: &CodeBlock, index: u8) {
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
		let id = ui.id().with(("code-block", index));
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
									Self::copy_button(ui, id, &block.code);
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
					let response = ui.add(egui::Label::new(job).wrap().selectable(true));
					response.widget_info(|| {
						egui::WidgetInfo::labeled(
							egui::WidgetType::Label,
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
						let mut child = ui.new_child(
							egui::UiBuilder::new()
								.max_rect(target)
								.layout(egui::Layout::left_to_right(egui::Align::Center)),
						);
						child.painter().rect_filled(target, 6, colors.raised);
						Self::copy_button(&mut child, id, &block.code);
					}
				}
				ui.add_space(4.0);
			},
		);
	}
	fn copied_recently(ui: &egui::Ui, id: egui::Id) -> bool {
		let now = ui.input(|input| input.time);
		ui.data(|data| data.get_temp::<f64>(id))
			.is_some_and(|at| now - at < 1.5)
	}
	fn copy_button(ui: &mut egui::Ui, id: egui::Id, code: &str) {
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
	}
	/// One galley per run: emoji occupy fixed-width slots inside the text layout, so rows
	/// holding artwork grow before any text on them is positioned. Separate widgets would
	/// leave text placed earlier on the row misaligned with text placed after the emoji.
	fn show_emoji(
		spans: &[(String, Style)],
		ui: &mut egui::Ui,
		link: bool,
		images: &mut crate::avatars::Avatars,
		demo: bool,
		guilds: &[model::Guild],
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
		let mut source = String::new();
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
					source.push_str(&text[start..offset]);
				}
				// One zero-width glyph plus leading space forms an unbroken inline slot; its
				// character is expanded to the wire text below so selection copies the original.
				job.append(
					"\u{200b}",
					size,
					TextFormat {
						color: egui::Color32::TRANSPARENT,
						line_height: Some(size),
						..format.clone()
					},
				);
				inlines.push(Inline {
					text: cluster.to_owned(),
					custom: custom.map(|(id, _)| id),
					image: cell.and_then(|cell| {
						atlas
							.get_or_insert_with(|| crate::emoji::atlas(ui.ctx()))
							.map(|atlas| crate::emoji::image_cell(atlas, cluster, cell, size))
					}),
				});
				source.push_str(cluster);
				offset += len;
				start = offset;
			}
			if start < text.len() {
				job.append(&text[start..], 0.0, format);
				source.push_str(&text[start..]);
			}
		}
		let mut label = egui::Label::new(job).wrap().selectable(true);
		if link {
			label = label.sense(egui::Sense::click());
		}
		let (pos, mut galley, mut response) = label.layout_in_ui(ui);
		response.widget_info(|| {
			egui::WidgetInfo::labeled(egui::WidgetType::Label, ui.is_enabled(), &source)
		});
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
						.any(|glyph| glyph.chr == '\u{200b}' && glyph.line_height == size)
				{
					continue;
				}
				let row = std::sync::Arc::make_mut(&mut placed.row);
				let mut glyphs = Vec::with_capacity(row.glyphs.len());
				for glyph in &row.glyphs {
					// Placeholders are the only glyphs with the artwork line height; a literal
					// zero-width space in message text keeps the body font's row height.
					if next >= inlines.len() || glyph.chr != '\u{200b}' || glyph.line_height != size
					{
						glyphs.push(*glyph);
						continue;
					}
					let index = next;
					next += 1;
					let left = glyph.pos.x - size;
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
		if ui.is_rect_visible(response.rect) {
			egui::text_selection::LabelSelectionState::label_text_selection(
				ui,
				&response,
				pos,
				galley,
				ui.visuals().text_color(),
				Stroke::NONE,
			);
			for (index, rect) in &slots {
				if !ui.is_rect_visible(*rect) {
					continue;
				}
				let inline = &inlines[*index];
				let image = match inline.custom {
					Some(id) => images.custom_image(ui.ctx(), id, size, demo),
					None => inline.image.clone(),
				};
				if let Some(image) = &image {
					let painted = image.calc_size(egui::Vec2::splat(size), image.size());
					image.paint_at(ui, egui::Rect::from_center_size(rect.center(), painted));
				} else {
					ui.painter().text(
						rect.center(),
						egui::Align2::CENTER_CENTER,
						"?",
						egui::FontId::proportional(size),
						ui.visuals().weak_text_color(),
					);
				}
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
							egui::WidgetType::Button,
							ui.is_enabled(),
							format!("Show emoji details: {}", inline.text),
						)
					});
					crate::emoji_details::show(ui, &hit, &inline.text, image, guilds);
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
			colors.mention_text
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
			} else if style.code {
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
			("> quoted\nplain", "│ quoted\nplain"),
			("> one\n> two", "│ one\n│ two"),
			(">>> all\nof\n\nthis", "│ all\n│ of\n\n│ this"),
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
						parsed.show_references(
							ui,
							&mut opening,
							&[],
							&mut profile,
							(&[], &mut channel, &[]),
							(&mut images, false, mask),
						)
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
					parsed.show_references(
						ui,
						&mut None,
						&[],
						&mut None,
						(&[], &mut None, &[]),
						(&mut images, false, &mut mask),
					)
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
						parsed.show_references(
							ui,
							&mut opening,
							&[],
							&mut profile,
							(&channels, &mut channel, &[]),
							(&mut crate::avatars::Avatars::default(), true, &mut revealed),
						)
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
					egui::Shape::Text(text) if text.galley.job.text != "?" => {
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
	fn mention_highlights_include_unknown_users_in_both_themes() {
		let ctx = egui::Context::default();
		let users = vec![model::User {
			id: Id(42),
			name: "Synthetic Robin".into(),
			avatar: None,
			webhook: false,
			kind: Default::default(),
			discriminator: 0,
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
