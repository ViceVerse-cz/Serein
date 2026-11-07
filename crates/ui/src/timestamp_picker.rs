//! One bounded composer timestamp dialog. Opening or confirming it never sends a message.
use crate::{dialog, i18n::translate, mentions};
use model::Id;
use std::{
	hash::{Hash, Hasher},
	ops::Range,
};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) struct Target {
	pub generation: u64,
	pub channel: Id,
	pub editing: Option<Id>,
}

struct Request {
	target: Target,
	range: Range<usize>,
	expected: String,
	fingerprint: u64,
	date: String,
	clock: String,
	offset: String,
	style: Option<u8>,
	rejected: bool,
}

#[derive(Default)]
pub(crate) struct Picker {
	request: Option<Request>,
}

pub(crate) fn fingerprint(draft: &str) -> u64 {
	let mut hash = std::collections::hash_map::DefaultHasher::new();
	draft.hash(&mut hash);
	hash.finish()
}

impl Picker {
	pub fn is_open(&self) -> bool {
		self.request.is_some()
	}

	pub fn accept(
		&mut self,
		draft: &mut String,
		pick: mentions::Pick,
		target: Target,
		remaining: usize,
	) -> Option<usize> {
		if let Some(range) = pick.time_range() {
			let expected = draft.get(range.clone())?.to_owned();
			let now = crate::local_time::now();
			let offset = now.offset().whole_seconds();
			self.request = Some(Request {
				target,
				range,
				expected,
				fingerprint: fingerprint(draft),
				date: format!(
					"{:04}-{:02}-{:02}",
					now.year(),
					u8::from(now.month()),
					now.day()
				),
				clock: format!("{:02}:{:02}:{:02}", now.hour(), now.minute(), now.second()),
				offset: format!(
					"{}{:02}:{:02}",
					if offset < 0 { '-' } else { '+' },
					offset.abs() / 3600,
					offset.abs() % 3600 / 60
				),
				style: None,
				rejected: false,
			});
			return None;
		}
		let cursor = mentions::insert(draft, pick, target.editing.is_some(), remaining);
		if cursor.is_some() {
			self.request = None;
		} else if let Some(request) = &mut self.request {
			request.rejected = true;
		}
		cursor
	}

	/// Drops the dialog when navigation no longer points at its original composer.
	pub fn retain(&mut self, target: Option<Target>) {
		if self
			.request
			.as_ref()
			.is_some_and(|r| Some(r.target) != target)
		{
			self.request = None;
		}
	}

	pub fn show(&mut self, ctx: &egui::Context, target: Target) -> Option<mentions::Pick> {
		self.retain(Some(target));
		let request = self.request.as_mut()?;
		let mut picked = None;
		let mut cancel = false;
		let response =
			dialog::Dialog::new("composer-timestamp", translate("timestamp-picker-title"))
				.icon(crate::icons::Icon::Calendar)
				.width(400.0)
				.show(ctx, |d| {
					d.content(|ui| {
						for (label, value, limit) in [
							("timestamp-picker-date", &mut request.date, 10),
							("timestamp-picker-clock", &mut request.clock, 8),
							("timestamp-picker-offset", &mut request.offset, 6),
						] {
							ui.label(translate(label));
							ui.add(
								egui::TextEdit::singleline(value)
									.id_salt(label)
									.char_limit(limit)
									.desired_width(f32::INFINITY),
							);
						}
						ui.weak(translate("timestamp-picker-offset-hint"));
						ui.label(translate("timestamp-picker-format"));
						egui::ComboBox::from_id_salt("timestamp-style")
							.selected_text(translate(style_key(request.style)))
							.show_ui(ui, |ui| {
								for style in [
									None,
									Some(b't'),
									Some(b'T'),
									Some(b'd'),
									Some(b'D'),
									Some(b'f'),
									Some(b'F'),
									Some(b's'),
									Some(b'S'),
									Some(b'R'),
								] {
									ui.selectable_value(
										&mut request.style,
										style,
										translate(style_key(style)),
									);
								}
							});
						if let Some(seconds) = parse(&request.date, &request.clock, &request.offset)
						{
							if request.style == Some(b'R') {
								ui.ctx()
									.request_repaint_after(crate::local_time::TIMESTAMP_REFRESH);
							}
							ui.label(
								crate::local_time::discord_timestamp(
									seconds,
									request.style.unwrap_or(b'f'),
								)
								.unwrap_or_default(),
							);
							ui.monospace(token(seconds, request.style));
						} else {
							ui.colored_label(
								crate::design::palette(ui).danger,
								translate("timestamp-picker-invalid"),
							);
						}
						if request.rejected {
							ui.colored_label(
								crate::design::palette(ui).danger,
								translate("timestamp-picker-rejected"),
							);
						}
					});
					d.footer(|ui| {
						cancel = dialog::action(
							ui,
							&translate("timestamp-picker-cancel"),
							dialog::Action::Neutral,
						)
						.clicked();
						let seconds = parse(&request.date, &request.clock, &request.offset);
						let insert = ui
							.add_enabled_ui(seconds.is_some(), |ui| {
								dialog::action(
									ui,
									&translate("timestamp-picker-insert"),
									dialog::Action::Primary,
								)
							})
							.inner
							.clicked();
						if insert && let Some(seconds) = seconds {
							picked = Some(mentions::Pick::timestamp(
								request.range.clone(),
								request.expected.clone(),
								request.fingerprint,
								token(seconds, request.style),
							));
						}
					});
				});
		if cancel || response.close {
			self.request = None;
		}
		picked
	}
}

fn style_key(style: Option<u8>) -> &'static str {
	match style {
		None => "timestamp-style-default",
		Some(b't') => "timestamp-style-t",
		Some(b'T') => "timestamp-style-long-time",
		Some(b'd') => "timestamp-style-d",
		Some(b'D') => "timestamp-style-long-date",
		Some(b'f') => "timestamp-style-f",
		Some(b'F') => "timestamp-style-full",
		Some(b's') => "timestamp-style-short-date-time",
		Some(b'S') => "timestamp-style-short-date-seconds",
		_ => "timestamp-style-relative",
	}
}

fn token(seconds: i64, style: Option<u8>) -> String {
	match style {
		Some(style) => format!("<t:{seconds}:{}>", char::from(style)),
		None => format!("<t:{seconds}>"),
	}
}

fn parse(date: &str, clock: &str, offset: &str) -> Option<i64> {
	if date.len() != 10
		|| !date.is_ascii()
		|| clock.len() != 8
		|| !clock.is_ascii()
		|| offset.len() != 6
		|| !offset.is_ascii()
	{
		return None;
	}
	if &date[4..5] != "-"
		|| &date[7..8] != "-"
		|| &clock[2..3] != ":"
		|| &clock[5..6] != ":"
		|| &offset[3..4] != ":"
		|| !matches!(&offset[..1], "+" | "-")
	{
		return None;
	}
	if !date
		.bytes()
		.enumerate()
		.all(|(i, b)| matches!(i, 4 | 7) || b.is_ascii_digit())
		|| !clock
			.bytes()
			.enumerate()
			.all(|(i, b)| matches!(i, 2 | 5) || b.is_ascii_digit())
		|| !offset
			.bytes()
			.enumerate()
			.all(|(i, b)| matches!(i, 0 | 3) || b.is_ascii_digit())
	{
		return None;
	}
	let date = time::Date::from_calendar_date(
		date[..4].parse().ok()?,
		time::Month::try_from(date[5..7].parse::<u8>().ok()?).ok()?,
		date[8..].parse().ok()?,
	)
	.ok()?;
	let clock = time::Time::from_hms(
		clock[..2].parse().ok()?,
		clock[3..5].parse().ok()?,
		clock[6..].parse().ok()?,
	)
	.ok()?;
	let hours: i32 = offset[1..3].parse().ok()?;
	let minutes: i32 = offset[4..].parse().ok()?;
	if hours > 23 || minutes > 59 {
		return None;
	}
	let seconds = (hours * 3600 + minutes * 60) * if offset.starts_with('-') { -1 } else { 1 };
	let offset = time::UtcOffset::from_whole_seconds(seconds).ok()?;
	let seconds = date
		.with_time(clock)
		.assume_utc()
		.unix_timestamp()
		.checked_sub(i64::from(offset.whole_seconds()))?;
	time::OffsetDateTime::from_unix_timestamp(seconds).ok()?;
	Some(seconds)
}

pub(crate) fn silent_indicator(ui: &mut egui::Ui) {
	let colors = crate::design::palette(ui);
	ui.horizontal_wrapped(|ui| {
		let (rect, _) = ui.allocate_exact_size(egui::Vec2::splat(14.0), egui::Sense::hover());
		crate::icons::paint(ui.painter(), crate::icons::Icon::Bell, rect, colors.muted);
		ui.painter().line_segment(
			[rect.left_bottom(), rect.right_top()],
			egui::Stroke::new(1.5, colors.muted),
		);
		ui.label(
			egui::RichText::new(translate("message-silent-indicator"))
				.small()
				.color(colors.muted),
		)
		.on_hover_text(translate("message-silent-hint"));
	});
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn civil_times_require_explicit_valid_offsets_and_calendar_values() {
		assert_eq!(parse("1970-01-01", "05:45:00", "+05:45"), Some(0));
		assert_eq!(parse("1970-01-01", "00:00:00", "-00:30"), Some(1800));
		assert!(parse("2000-02-29", "12:00:00", "+00:00").is_some());
		for (date, clock, offset) in [
			("1900-02-29", "12:00:00", "+00:00"),
			("9999-12-31", "23:59:59", "-23:59"),
			("2026-11-06", "24:00:00", "+00:00"),
			("2026-11-06", "12:00:00", "+00:60"),
			("2026-11-06", "12:00:00", "+-1:00"),
			("2026-11-06", "12:00:00", "+00:-1"),
			("2026-+1-06", "12:00:00", "+00:00"),
			("2026-11-06", "１２:00:00", "+00:00"),
		] {
			assert_eq!(parse(date, clock, offset), None, "{date} {clock} {offset}");
		}
	}

	#[test]
	fn timestamp_replacement_preserves_unicode_and_rejects_stale_or_full_drafts() {
		let original = "🙂 @time tail";
		let start = original.find('@').unwrap();
		let pick = || {
			mentions::Pick::timestamp(
				start..start + 5,
				"@time".into(),
				fingerprint(original),
				token(0, Some(b'R')),
			)
		};
		let mut draft = original.to_owned();
		assert_eq!(mentions::insert(&mut draft, pick(), false, 64), Some(10));
		assert_eq!(draft, "🙂 <t:0:R>  tail");
		assert_eq!(model::timestamp_prefix(&draft[start..]), Some((0, b'R', 7)));
		let mut stale = original.replace("tail", "edit");
		let unchanged = stale.clone();
		assert_eq!(mentions::insert(&mut stale, pick(), false, 64), None);
		assert_eq!(stale, unchanged);
		let mut full = String::with_capacity(original.len());
		full.push_str(original);
		assert_eq!(mentions::insert(&mut full, pick(), false, 0), None);
		assert_eq!(full, original);
	}
}
