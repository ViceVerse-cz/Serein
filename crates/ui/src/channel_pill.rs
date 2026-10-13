//! Channel and message destinations use the same metadata and bundled icons.
use crate::{icons::Icon, mentions::MentionSource, select::Surface};
use model::{Channel, Guild, Id};
use std::{borrow::Cow, sync::Arc};

/// Corner radius shared by channel, user and role mention backgrounds.
pub(crate) const RADIUS: f32 = 3.0;
/// Horizontal padding inside mention backgrounds, so adjacent mentions stay apart.
const PAD: f32 = 4.0;
/// Space after a mention, so mentions written without a space between them stay apart.
const GAP: f32 = 3.0;

/// Bound displayed destination names; the original URL is never shortened.
const MAX_NAME_BYTES: usize = 192;
const MAX_NAME_SCAN: usize = 256;

pub(crate) fn icon(channel: &Channel, channels: &[Channel]) -> Icon {
	match channel.kind {
		1 => Icon::Profile,
		3 => Icon::People,
		2 | 13 => Icon::Speaker,
		5 => Icon::Megaphone,
		15 | 16 => Icon::Threads,
		10..=12 if forum_parent(channel, channels).is_some() => Icon::Forum,
		10..=12 => Icon::Thread,
		_ => Icon::Hash,
	}
}

fn forum_parent<'a>(channel: &Channel, channels: &'a [Channel]) -> Option<&'a Channel> {
	channels.iter().find(|parent| {
		Some(parent.id) == channel.parent_id
			&& parent.id != channel.id
			&& parent.guild == channel.guild
			&& parent.guild.is_some()
			&& matches!(parent.kind, 15 | 16)
	})
}

pub(crate) struct Pill<'a> {
	pub icon: Icon,
	pub name: Cow<'a, str>,
	pub message: bool,
	pub post: Option<Cow<'a, str>>,
	pub guild: Option<&'a Guild>,
}

impl<'a> Pill<'a> {
	pub fn channel(
		id: Id,
		channels: &'a [Channel],
		source: Option<&MentionSource<'a>>,
	) -> Option<Self> {
		let channel = channels.iter().find(|channel| channel.id == id);
		// A cached channel shows its name and kind only if the session may view and read
		// it. Retained reference names belong to threads this session loaded itself.
		let readable = |channel: &Channel| {
			source.is_none_or(|source| {
				source.state.can_view(channel.id) && source.state.can_read_history(channel.id)
			})
		};
		let (name, channel) = match channel {
			Some(channel)
				if channel.guild.is_some() && matches!(channel.kind, 0 | 5 | 10..=12 | 15 | 16) =>
			{
				if readable(channel) {
					(channel.name.as_str(), Some(channel))
				} else {
					("unknown-channel", None)
				}
			}
			Some(_) => return None,
			None => (
				source
					.and_then(|source| source.state.channel_reference_name(id))
					.unwrap_or("unknown-channel"),
				None,
			),
		};
		Some(Self {
			icon: channel.map_or(Icon::Hash, |channel| icon(channel, channels)),
			name: bounded_name(name, "unknown-channel"),
			message: false,
			post: None,
			guild: None,
		})
	}

	/// A message or channel link. Names come only from destinations the session may view
	/// and read; hidden or mismatched targets stay generic. A joined server's name is not
	/// private.
	pub fn message(
		link: &crate::markdown::ChatLink,
		channels: &'a [Channel],
		guilds: &'a [Guild],
		source: Option<&MentionSource<'a>>,
	) -> Self {
		let message = link.message.is_some();
		let current_guild = source.and_then(|source| {
			source
				.state
				.channel(source.channel)
				.and_then(|channel| channel.guild)
		});
		if link.guild != current_guild
			&& let Some(guild) = guilds.iter().find(|guild| Some(guild.id) == link.guild)
		{
			return Self {
				icon: Icon::Servers,
				name: bounded_name(&guild.name, "unknown-server"),
				message,
				post: None,
				guild: Some(guild),
			};
		}
		let readable = |channel: &Channel| {
			source.is_some_and(|source| {
				source.state.can_view(channel.id) && source.state.can_read_history(channel.id)
			})
		};
		let channel = channels.iter().find(|channel| {
			channel.id == link.channel
				&& channel.guild == link.guild
				&& (matches!(channel.kind, 0 | 2 | 5 | 10..=12 | 13 | 15 | 16)
					&& channel.guild.is_some()
					|| matches!(channel.kind, 1 | 3) && channel.guild.is_none())
				&& readable(channel)
		});
		// A message in a forum post reads `forum > post`; a link to the post itself is
		// just the post.
		let parent = channel
			.filter(|channel| message && matches!(channel.kind, 10..=12))
			.and_then(|channel| forum_parent(channel, channels))
			.filter(|parent| readable(parent));
		let fallback = if link.guild.is_some() {
			"unknown-channel"
		} else {
			"unknown-conversation"
		};
		let Some(channel) = channel else {
			return Self {
				icon: if link.guild.is_some() {
					Icon::Hash
				} else {
					Icon::Profile
				},
				name: Cow::Borrowed(fallback),
				message,
				post: None,
				guild: None,
			};
		};
		let name = match (parent, source) {
			(Some(parent), _) => parent.name.as_str(),
			(None, Some(source)) => source.state.conversation_name(channel),
			(None, None) => channel.name.as_str(),
		};
		Self {
			icon: icon(parent.unwrap_or(channel), channels),
			name: bounded_name(name, fallback),
			message,
			post: parent.map(|_| bounded_name(&channel.name, fallback)),
			guild: None,
		}
	}

	/// Localized semantic equivalent of the leading icon, also used by reply previews.
	pub fn prefix(&self) -> String {
		let key = match self.icon {
			Icon::Hash | Icon::Megaphone | Icon::Speaker => return "#".into(),
			Icon::Thread => "channel-pill-thread",
			Icon::Threads => "channel-pill-forum",
			Icon::Forum => "channel-pill-post",
			_ => return String::new(),
		};
		format!("{}: ", crate::i18n::translate(key))
	}

	/// Accessible destination text; names and breadcrumb symbols remain unchanged.
	pub fn label(&self) -> String {
		format!("{}{}", self.prefix(), self.text())
	}

	/// Destination text after the leading channel or server icon.
	pub fn text(&self) -> String {
		let mut label = self.name.clone().into_owned();
		if self.message {
			label.push_str(" > ");
			if let Some(post) = &self.post {
				label.push_str(post);
			} else {
				label.push_str(&crate::i18n::translate("channel-pill-message"));
			}
		}
		label
	}

	/// With `copy`, selecting across the pill copies the original URL instead of the
	/// resolved name; otherwise the caller keeps the pill out of text selection.
	pub fn show(
		&self,
		ui: &mut egui::Ui,
		images: &mut crate::avatars::Avatars,
		demo: bool,
		line: Option<f32>,
		copy: Option<(&mut Surface, &str)>,
	) -> egui::Response {
		let colors = crate::design::palette(ui);
		let font_id = egui::TextStyle::Body.resolve(ui.style());
		let size = font_id.size;
		let text_height = ui.fonts_mut(|fonts| fonts.row_height(&font_id));
		let format = egui::TextFormat {
			font_id,
			color: colors.mention_text,
			valign: ui.text_valign(),
			line_height: line,
			..Default::default()
		};
		// Each icon sits on a body-font space; the following section's leading space
		// widens it to the icon size. A separate slot font would change the row's
		// ascent and push the text below its neighbours.
		let space = ui.fonts_mut(|fonts| fonts.glyph_width(&format.font_id, ' '));
		let widen = (size - space).max(0.0);
		let mut job = egui::text::LayoutJob::default();
		// Label sets wrap indentation on the first section.
		job.append("", 0.0, format.clone());
		job.append(" ", PAD, format.clone());
		job.append(&format!(" {}", self.name), widen, format.clone());
		let trailing = job.text.chars().count();
		// Without a post name, a final space carries the message glyph's width.
		let mut trim = 0.0;
		if self.message {
			job.append(" ", 0.0, format.clone());
			job.append(" ", widen, format.clone());
			match &self.post {
				Some(post) => job.append(&format!(" {post}"), widen, format),
				None => {
					job.append(" ", widen, format);
					trim = space;
				}
			}
		}
		let (pos, galley, response) = egui::Label::new(job)
			.wrap()
			.selectable(false)
			.sense(egui::Sense::hover())
			.layout_in_ui(ui);
		let id = response.id.with("destination");
		let rows = backgrounds(&galley, pos, text_height, trim);
		let response = interact_rows(ui, id, &rows, egui::Sense::click(), copy).unwrap_or(response);
		response.widget_info(|| {
			egui::WidgetInfo::labeled(egui::Role::Link, ui.is_enabled(), self.label())
		});
		if ui.is_rect_visible(response.rect) {
			let fill = fill(&response, colors.mention_bg, colors.mention_text);
			for rect in &rows {
				ui.painter().rect_filled(*rect, RADIUS, fill);
			}
			ui.painter().add(egui::epaint::TextShape::new(
				pos,
				galley.clone(),
				colors.mention_text,
			));
			for (index, at) in [0, trailing, trailing + 1]
				.into_iter()
				.take(if self.message { 3 } else { 1 })
				.enumerate()
			{
				let mut cursor = egui::text::CCursor::new(at);
				cursor.prefer_next_row = true;
				let position = galley.pos_from_cursor(cursor).translate(pos.to_vec2());
				let rect = egui::Rect::from_center_size(
					egui::pos2(position.left() + size / 2.0, position.center().y),
					egui::Vec2::splat(size),
				);
				if index == 0
					&& let Some(guild) = self.guild
				{
					images.paint_guild(ui, guild, rect, demo, 3);
				} else {
					let icon = [self.icon, Icon::ChevronRight, Icon::Forum][index];
					crate::icons::paint(ui.painter(), icon, rect.shrink(1.0), colors.mention_text);
				}
			}
		}
		ui.add_space(PAD + GAP);
		response.on_hover_cursor(egui::CursorIcon::PointingHand)
	}
}

/// Inline mention text on a rounded background, in the body font and wrapping like text.
/// Each wrapped row gets its own background, as for channel pills.
/// With `copy`, selecting across the mention copies `text`; otherwise the caller keeps it
/// out of text selection.
pub(crate) fn mention(
	ui: &mut egui::Ui,
	text: &str,
	color: egui::Color32,
	background: egui::Color32,
	sense: egui::Sense,
	line: Option<f32>,
	copy: Option<&mut Surface>,
) -> egui::Response {
	let font_id = egui::TextStyle::Body.resolve(ui.style());
	let text_height = ui.fonts_mut(|fonts| fonts.row_height(&font_id));
	let format = egui::TextFormat {
		font_id,
		color,
		valign: ui.text_valign(),
		line_height: line,
		..Default::default()
	};
	let mut job = egui::text::LayoutJob::default();
	// Label sets wrap indentation on the first section.
	job.append("", 0.0, format.clone());
	job.append(text, PAD, format);
	let (pos, galley, response) = egui::Label::new(job)
		.wrap()
		.selectable(false)
		.sense(egui::Sense::hover())
		.layout_in_ui(ui);
	let rows = backgrounds(&galley, pos, text_height, 0.0);
	let id = response.id.with("mention");
	let copy = copy.map(|surface| (surface, text));
	let response = interact_rows(ui, id, &rows, sense, copy).unwrap_or(response);
	if ui.is_rect_visible(response.rect) {
		let fill = fill(&response, background, color);
		for rect in rows {
			ui.painter().rect_filled(rect, RADIUS, fill);
		}
		ui.painter()
			.add(egui::epaint::TextShape::new(pos, galley, color));
	}
	ui.add_space(PAD + GAP);
	response
}

/// One interaction per wrapped row so a bounding box cannot steal neighbours' clicks;
/// only the first row is a keyboard stop. With `copy`, the whole text occupies one
/// unbroken selection slot over the first row.
fn interact_rows(
	ui: &mut egui::Ui,
	id: egui::Id,
	rows: &[egui::Rect],
	sense: egui::Sense,
	copy: Option<(&mut Surface, &str)>,
) -> Option<egui::Response> {
	let hits: Vec<_> = rows
		.iter()
		.enumerate()
		.map(|(index, rect)| {
			let sense = if index == 0 {
				sense
			} else {
				sense - egui::Sense::FOCUSABLE
			};
			ui.interact(*rect, id.with(index), sense)
		})
		.collect();
	if let Some((surface, text)) = copy
		&& let Some(first) = hits.first()
	{
		surface.run(
			ui,
			first,
			first.rect.min,
			selection_galley(ui, text, first.rect.size()),
			vec![],
		);
		for hit in &hits {
			surface.through(hit);
		}
	}
	hits.into_iter().reduce(|response, hit| response.union(hit))
}

/// Hover and keyboard focus fill the background more, instead of underlining.
fn fill(
	response: &egui::Response,
	background: egui::Color32,
	text: egui::Color32,
) -> egui::Color32 {
	if response.hovered() || response.has_focus() {
		background.lerp_to_gamma(text, 0.25)
	} else {
		background
	}
}

/// One background per wrapped row: a body-text line centered on the row, plus
/// horizontal padding. A taller first row (set by preceding content) stays outside it.
/// `trim` removes a trailing placeholder space from the last row.
fn backgrounds(
	galley: &egui::Galley,
	pos: egui::Pos2,
	text_height: f32,
	trim: f32,
) -> Vec<egui::Rect> {
	let last = galley.rows.len().saturating_sub(1);
	galley
		.rows
		.iter()
		.enumerate()
		.filter(|(_, row)| !row.glyphs.is_empty())
		.map(|(index, row)| {
			let rect = row.rect_without_leading_space().translate(pos.to_vec2());
			let right = rect.right() - if index == last { trim } else { 0.0 };
			egui::Rect::from_x_y_ranges(
				rect.left() - PAD..=right + PAD,
				rect.center().y - text_height / 2.0..=rect.center().y + text_height / 2.0,
			)
		})
		.collect()
}

/// An invisible galley whose text is `url` (or a mention), every character sharing one
/// hit slot.
fn selection_galley(ui: &egui::Ui, url: &str, size: egui::Vec2) -> Arc<egui::Galley> {
	use egui::{Color32, text::LayoutJob};
	let font = egui::TextStyle::Body.resolve(ui.style());
	let mut galley = ui.fonts_mut(|fonts| {
		fonts.layout_job(LayoutJob::simple(
			" ".into(),
			font.clone(),
			Color32::TRANSPARENT,
			f32::INFINITY,
		))
	});
	let galley_mut = Arc::make_mut(&mut galley);
	let placed = &mut galley_mut.rows[0];
	let row = Arc::make_mut(&mut placed.row);
	let slot = row.glyphs[0];
	row.glyphs = url
		.chars()
		.map(|chr| {
			let mut glyph = slot;
			glyph.chr = chr;
			glyph.pos.x = 0.0;
			glyph.advance_width = size.x;
			glyph.line_height = size.y;
			glyph
		})
		.collect();
	row.size = size;
	galley_mut.rect = egui::Rect::from_min_size(egui::Pos2::ZERO, size);
	galley_mut.job = Arc::new(LayoutJob::simple(
		url.into(),
		font,
		Color32::TRANSPARENT,
		f32::INFINITY,
	));
	galley
}

/// Cached names are untrusted single-line labels: drop bidi and invisible controls,
/// collapse whitespace, and bound both scanning and output.
fn bounded_name<'a>(value: &'a str, fallback: &'static str) -> Cow<'a, str> {
	let invisible = |chr: char| {
		matches!(chr, '\u{061c}' | '\u{200b}'..='\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}' | '\u{feff}')
			|| chr.is_control()
	};
	if !value.is_empty()
		&& value.len() <= MAX_NAME_BYTES
		&& !value.starts_with(char::is_whitespace)
		&& !value.ends_with(char::is_whitespace)
		&& !value.contains("  ")
		&& !value
			.chars()
			.any(|chr| invisible(chr) || chr.is_whitespace() && chr != ' ')
	{
		return Cow::Borrowed(value);
	}
	let mut name = String::new();
	let mut consumed = 0;
	for (at, chr) in value.char_indices().take(MAX_NAME_SCAN) {
		consumed = at + chr.len_utf8();
		if invisible(chr) && !chr.is_whitespace() {
			continue;
		}
		let chr = if chr.is_whitespace() { ' ' } else { chr };
		if chr == ' ' && (name.is_empty() || name.ends_with(' ')) {
			continue;
		}
		if name.len() + chr.len_utf8() > MAX_NAME_BYTES - '…'.len_utf8() {
			consumed = at;
			break;
		}
		name.push(chr);
	}
	let mut name = name.trim_end().to_owned();
	if name.is_empty() {
		return Cow::Borrowed(fallback);
	}
	if consumed < value.len() {
		name.push('…');
	}
	Cow::Owned(name)
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn destinations_distinguish_channels_threads_posts_and_foreign_servers() {
		let mut state = test_support::demo_state();
		let source = MentionSource {
			state: &state,
			channel: Id(20),
		};
		for (id, expected) in [
			(20, Icon::Hash),
			(28, Icon::Thread),
			(26, Icon::Threads),
			(27, Icon::Forum),
		] {
			let pill = Pill::channel(Id(id), &state.channels, Some(&source)).unwrap();
			assert_eq!(pill.icon, expected);
			assert!(!pill.message && pill.post.is_none() && pill.guild.is_none());
			let link = crate::markdown::ChatLink {
				guild: Some(Id(10)),
				channel: Id(id),
				message: Some(Id(100)),
			};
			let pill = Pill::message(&link, &state.channels, &state.guilds, Some(&source));
			assert!(pill.message);
			if id == 27 {
				assert_eq!(pill.icon, Icon::Threads);
				assert_eq!(pill.name, state.channel(Id(26)).unwrap().name);
				assert_eq!(
					pill.post.as_deref(),
					Some(state.channel(Id(27)).unwrap().name.as_str())
				);
			} else {
				assert_eq!(pill.icon, expected);
				assert!(pill.post.is_none());
			}
		}
		let mut other = state.guilds[0].clone();
		other.id = Id(999);
		other.name = "Other server".into();
		state.guilds.push(other);
		let source = MentionSource {
			state: &state,
			channel: Id(20),
		};
		let link = crate::markdown::ChatLink {
			guild: Some(Id(999)),
			channel: Id(9999),
			message: Some(Id(100)),
		};
		let pill = Pill::message(&link, &state.channels, &state.guilds, Some(&source));
		assert_eq!(pill.name, "Other server");
		assert_eq!(pill.guild.map(|guild| guild.id), Some(Id(999)));
		assert_eq!(pill.label(), "Other server > message");
		let biography = Pill::message(&link, &[], &state.guilds, None);
		assert_eq!(biography.guild.map(|guild| guild.id), Some(Id(999)));
		assert_eq!(biography.label(), "Other server > message");
	}

	#[test]
	fn missing_or_mismatched_metadata_never_borrows_an_unrelated_name() {
		let mut state = test_support::demo_state();
		let link = crate::markdown::ChatLink {
			guild: Some(Id(999)),
			channel: Id(20),
			message: Some(Id(100)),
		};
		assert_eq!(
			Pill::message(&link, &state.channels, &state.guilds, None).name,
			"unknown-channel"
		);
		assert!(Pill::channel(Id(22), &state.channels, None).is_none());
		assert_eq!(
			Pill::channel(Id(999), &state.channels, None).unwrap().name,
			"unknown-channel"
		);
		let post = state
			.channels
			.iter_mut()
			.find(|channel| channel.id == Id(27))
			.unwrap();
		post.guild = Some(Id(999));
		assert_eq!(icon(post, &[]), Icon::Thread);
		assert_eq!(
			icon(state.channel(Id(27)).unwrap(), &state.channels),
			Icon::Thread
		);
		let post = state
			.channels
			.iter_mut()
			.find(|channel| channel.id == Id(27))
			.unwrap();
		post.parent_id = Some(post.id);
		assert_eq!(
			icon(state.channel(Id(27)).unwrap(), &state.channels),
			Icon::Thread
		);
	}

	fn kinds_state() -> client_core::State {
		let mut state = test_support::demo_state();
		let base = state.channels[0].clone();
		state.channels = [
			(20, Some(10), 0, None, "orders"),
			(21, Some(10), 0, None, "project-updates"),
			(22, Some(10), 2, None, "voice-lounge"),
			(23, Some(10), 5, None, "announcements"),
			(24, Some(10), 15, None, "video-forum"),
			(25, Some(10), 11, Some(24), "YT-app"),
			(26, Some(10), 12, Some(21), "private-thread"),
			(28, None, 1, None, "Synthetic Robin"),
			(29, None, 3, None, "Weekend group"),
			(32, Some(10), 4, None, "category-not-a-conversation"),
		]
		.into_iter()
		.map(|(id, guild, kind, parent, name)| Channel {
			id: Id(id),
			guild: guild.map(Id),
			kind,
			parent_id: parent.map(Id),
			name: name.into(),
			..base.clone()
		})
		.collect();
		state.invalidate_navigation();
		state
			.permissions
			.replace(test_support::permission_snapshot(&state))
			.unwrap();
		state
	}

	fn resolve(state: &client_core::State, guild: Option<u64>, channel: u64) -> Pill<'_> {
		let link = crate::markdown::ChatLink {
			guild: guild.map(Id),
			channel: Id(channel),
			message: Some(Id(100)),
		};
		let source = MentionSource {
			state,
			channel: Id(20),
		};
		Pill::message(&link, &state.channels, &state.guilds, Some(&source))
	}

	#[test]
	fn message_links_name_voice_announcement_and_direct_conversations() {
		let state = kinds_state();
		for (guild, channel, icon, label) in [
			(Some(10), 22, Icon::Speaker, "#voice-lounge > message"),
			(Some(10), 23, Icon::Megaphone, "#announcements > message"),
			(Some(10), 25, Icon::Threads, "Forum: video-forum > YT-app"),
			(
				Some(10),
				26,
				Icon::Thread,
				"Thread: private-thread > message",
			),
			(None, 28, Icon::Profile, "Synthetic Robin > message"),
			(None, 29, Icon::People, "Weekend group > message"),
		] {
			let pill = resolve(&state, guild, channel);
			assert_eq!((pill.icon, pill.label().as_str()), (icon, label));
		}
	}

	#[test]
	fn hidden_or_mismatched_message_destinations_stay_generic() {
		use model::permissions as p;
		let state = kinds_state();
		for (guild, channel, icon, name) in [
			(Some(10), 999, Icon::Hash, "unknown-channel"),
			(None, 21, Icon::Profile, "unknown-conversation"),
			(Some(10), 28, Icon::Hash, "unknown-channel"),
			(Some(10), 32, Icon::Hash, "unknown-channel"),
		] {
			let pill = resolve(&state, guild, channel);
			assert_eq!((pill.icon, pill.name.as_ref()), (icon, name));
			assert!(pill.post.is_none() && pill.guild.is_none());
		}
		for deny in [p::VIEW_CHANNEL, p::READ_MESSAGE_HISTORY] {
			let mut state = kinds_state();
			let mut snapshot = test_support::permission_snapshot(&state);
			snapshot
				.channels
				.iter_mut()
				.find(|channel| channel.id == Id(21))
				.unwrap()
				.overwrites = Some(vec![p::Overwrite {
				id: Id(10),
				kind: 0,
				allow: 0,
				deny,
			}]);
			state.permissions.replace(snapshot).unwrap();
			// Names stay cached; the permission gate, not deletion, hides them.
			for target in [21, 26] {
				assert!(state.channel(Id(target)).is_some());
				let label = resolve(&state, Some(10), target).label();
				assert_eq!(label, "#unknown-channel > message");
				let source = MentionSource {
					state: &state,
					channel: Id(20),
				};
				let mention = Pill::channel(Id(target), &state.channels, Some(&source)).unwrap();
				assert_eq!(
					(mention.icon, mention.label()),
					(Icon::Hash, "#unknown-channel".into())
				);
			}
		}
	}

	#[test]
	fn cached_names_are_bounded_single_line_and_strip_spoofing_controls() {
		assert!(matches!(
			bounded_name("orders", "x"),
			Cow::Borrowed("orders")
		));
		assert_eq!(
			bounded_name("\u{202e}  orders\n\t desk\u{200b}\u{0}", "unknown-channel"),
			"orders desk"
		);
		assert_eq!(
			bounded_name(" \u{202e}\n", "unknown-channel"),
			"unknown-channel"
		);
		for value in [
			"界🦀".repeat(400),
			format!("{}\nsecret", "\u{202e}".repeat(500)),
		] {
			let name = bounded_name(&value, "unknown-channel");
			assert!(name.len() <= MAX_NAME_BYTES);
			assert!(!name.contains(['\n', '\u{202e}']));
		}
		assert!(bounded_name(&"界".repeat(400), "unknown-channel").ends_with('…'));
	}
}
