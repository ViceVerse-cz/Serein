//! Channel and message destinations use the same metadata and bundled icons.
use crate::{icons::Icon, mentions::MentionSource};
use model::{Channel, Guild, Id};

pub(crate) fn icon(channel: &Channel, channels: &[Channel]) -> Icon {
	match channel.kind {
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
	pub name: &'a str,
	pub message: bool,
	pub post: Option<&'a str>,
	pub guild: Option<&'a Guild>,
}

impl<'a> Pill<'a> {
	pub fn channel(
		id: Id,
		channels: &'a [Channel],
		source: Option<&MentionSource<'a>>,
	) -> Option<Self> {
		let channel = channels.iter().find(|channel| channel.id == id);
		let name = match channel {
			Some(channel)
				if channel.guild.is_some() && matches!(channel.kind, 0 | 5 | 10..=12 | 15 | 16) =>
			{
				channel.name.as_str()
			}
			Some(_) => return None,
			None => source
				.and_then(|source| source.state.channel_reference_name(id))
				.unwrap_or("unknown-channel"),
		};
		Some(Self {
			icon: channel.map_or(Icon::Hash, |channel| icon(channel, channels)),
			name,
			message: false,
			post: None,
			guild: None,
		})
	}

	pub fn message(
		link: &crate::markdown::ChatLink,
		channels: &'a [Channel],
		guilds: &'a [Guild],
		source: Option<&MentionSource<'a>>,
	) -> Self {
		let current_guild = source.and_then(|source| {
			source
				.state
				.channel(source.channel)
				.and_then(|channel| channel.guild)
		});
		if source.is_some()
			&& link.guild != current_guild
			&& let Some(guild) = guilds.iter().find(|guild| Some(guild.id) == link.guild)
		{
			return Self {
				icon: Icon::Servers,
				name: &guild.name,
				message: true,
				post: None,
				guild: Some(guild),
			};
		}
		let channel = channels.iter().find(|channel| {
			channel.id == link.channel
				&& channel.guild == link.guild
				&& (matches!(channel.kind, 0 | 5 | 10..=12 | 15 | 16) && channel.guild.is_some()
					|| matches!(channel.kind, 1 | 3) && channel.guild.is_none())
		});
		let parent = channel
			.filter(|channel| matches!(channel.kind, 10..=12))
			.and_then(|channel| forum_parent(channel, channels));
		Self {
			icon: parent
				.or(channel)
				.map_or(Icon::Hash, |channel| icon(channel, channels)),
			name: parent
				.or(channel)
				.map_or("unknown-channel", |channel| channel.name.as_str()),
			message: true,
			post: parent.and(channel).map(|channel| channel.name.as_str()),
			guild: None,
		}
	}

	pub fn label(&self) -> String {
		let prefix = match self.icon {
			Icon::Hash => "#",
			Icon::Thread => "Thread: ",
			Icon::Threads => "Forum: ",
			Icon::Forum => "Post: ",
			_ => "",
		};
		let mut label = format!("{prefix}{}", self.name);
		if self.message {
			label.push_str(" > ");
			label.push_str(self.post.unwrap_or("message"));
		}
		label
	}

	pub fn show(
		&self,
		ui: &mut egui::Ui,
		images: &mut crate::avatars::Avatars,
		demo: bool,
	) -> egui::Response {
		let colors = crate::design::palette(ui);
		let size = egui::TextStyle::Body.resolve(ui.style()).size;
		let format = egui::TextFormat {
			font_id: egui::FontId::new(size, crate::design::semibold_family(ui.ctx())),
			color: colors.mention_text,
			background: colors.mention_bg,
			..Default::default()
		};
		let mut slot = crate::emoji::inline_format(ui, size, size);
		slot.background = colors.mention_bg;
		let mut job = egui::text::LayoutJob::default();
		// Label sets wrap indentation on the first section.
		job.append("", 0.0, format.clone());
		job.append(" ", 0.0, slot.clone());
		job.append(&format!(" {}", self.name), 0.0, format.clone());
		let trailing = job.text.chars().count();
		if self.message {
			job.append(" ", 0.0, slot.clone());
			job.append(" ", 0.0, slot);
			if let Some(post) = self.post {
				job.append(&format!(" {post}"), 0.0, format);
			}
		}
		let (pos, galley, response) = egui::Label::new(job)
			.wrap()
			.selectable(false)
			.sense(egui::Sense::hover())
			.layout_in_ui(ui);
		let id = response.id.with("destination");
		let mut hits: Option<egui::Response> = None;
		for (index, row) in galley.rows.iter().enumerate() {
			let rect = row.rect_without_leading_space().translate(pos.to_vec2());
			let sense = if index == 0 {
				egui::Sense::click()
			} else {
				egui::Sense::click() - egui::Sense::FOCUSABLE
			};
			let hit = ui.interact(rect, id.with(index), sense);
			hits = Some(match hits {
				Some(response) => response.union(hit),
				None => hit,
			});
		}
		let response = hits.unwrap_or(response);
		response.widget_info(|| {
			egui::WidgetInfo::labeled(egui::Role::Link, ui.is_enabled(), self.label())
		});
		if ui.is_rect_visible(response.rect) {
			let underline = if response.hovered() || response.has_focus() {
				egui::Stroke::new(1.0, colors.mention_text)
			} else {
				egui::Stroke::NONE
			};
			ui.painter().add(
				egui::epaint::TextShape::new(pos, galley.clone(), colors.mention_text)
					.with_underline(underline),
			);
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
		response.on_hover_cursor(egui::CursorIcon::PointingHand)
	}
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
					pill.post,
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
}
