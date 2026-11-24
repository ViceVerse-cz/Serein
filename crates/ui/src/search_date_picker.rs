//! Calendar rows for the More Filters dialog, using the existing UTC search cursors.
use crate::{
	design,
	i18n::{translate, translate_count},
	icons,
};
use time::{Date, Duration, Month, OffsetDateTime};

const DISCORD_EPOCH_MS: i64 = 1_420_070_400_000;

pub(super) fn parse_date(value: &str) -> Option<Date> {
	let format = time::format_description::parse_borrowed::<2>("[year]-[month]-[day]").ok()?;
	Date::parse(value, &format).ok()
}

pub(super) fn day_start(date: Date) -> Option<u64> {
	let millis = date.midnight().assume_utc().unix_timestamp() * 1000;
	let delta = u64::try_from(millis - DISCORD_EPOCH_MS).ok()?;
	(1..=(u64::MAX >> 22))
		.contains(&delta)
		.then_some(delta << 22)
}

pub(super) fn date_at(id: u64) -> Option<Date> {
	let millis = (id >> 22) as i64 + DISCORD_EPOCH_MS;
	Some(
		OffsetDateTime::from_unix_timestamp(millis / 1000)
			.ok()?
			.date(),
	)
}

/// A date token can stand in for an ID only at the exact start of that UTC day.
pub(super) fn date_at_midnight(id: u64) -> Option<Date> {
	let date = date_at(id)?;
	(day_start(date) == Some(id)).then_some(date)
}

/// REST uses exclusive ID comparisons. Include midnight in the selected day.
pub(super) fn during_bounds(date: Date) -> Option<(u64, u64)> {
	Some((day_start(date)? - 1, day_start(date.next_day()?)?))
}

fn effective_bounds(query: &str) -> (Option<u64>, Option<u64>) {
	let mut after: Option<u64> = None;
	let mut before: Option<u64> = None;
	for (key, value) in query
		.split_whitespace()
		.filter_map(|token| token.split_once(':'))
	{
		match key {
			"after_id" => after = after.into_iter().chain(value.parse::<u64>().ok()).max(),
			"before_id" => before = before.into_iter().chain(value.parse::<u64>().ok()).min(),
			_ => {}
		}
	}
	(after, before)
}

/// Recognize the exact bounds emitted by During, including the inclusive midnight.
pub(super) fn during_date(query: &str) -> Option<Date> {
	let (after, before) = effective_bounds(query);
	let (after, before) = (after?, before?);
	let date = date_at(after.checked_add(1)?)?;
	(during_bounds(date) == Some((after, before))).then_some(date)
}

pub(super) fn is_bound(token: &str) -> bool {
	token.starts_with("after_id:") || token.starts_with("before_id:")
}

fn earliest() -> Date {
	Date::from_calendar_date(2015, Month::January, 2).expect("valid search minimum")
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
	Before,
	After,
	During,
}

impl Kind {
	fn label(self) -> String {
		translate(match self {
			Self::Before => "search-date-before",
			Self::After => "search-date-after",
			Self::During => "search-date-during",
		})
	}
}

struct Row {
	kind: Kind,
	date: Date,
	month: Date,
	cursor: Option<u64>,
}

impl Row {
	fn new(kind: Kind, date: Date) -> Self {
		Self {
			kind,
			date,
			month: month_start(date),
			cursor: None,
		}
	}

	fn show(&mut self, ui: &mut egui::Ui, today: Date, only_kind: bool) -> (bool, bool) {
		let previous = (self.kind, self.date);
		let remove = ui
			.horizontal(|ui| {
				let kind_width = (ui.available_width() * 0.32).clamp(84.0, 140.0);
				ui.add_enabled_ui(!only_kind, |ui| {
					design::Select::from_id_salt("date-kind")
						.width(kind_width)
						.selected_text(self.kind.label())
						.show_ui(ui, |ui| {
							for kind in [Kind::Before, Kind::After, Kind::During] {
								if ui
									.selectable_label(self.kind == kind, kind.label())
									.clicked()
								{
									self.kind = kind;
									ui.close();
								}
							}
						});
				});
				let width = ui.available_width() - 32.0 - ui.spacing().item_spacing.x;
				self.date_button(ui, width, today);
				icons::button(ui, icons::Icon::Trash, 32.0, "search-date-remove").clicked()
			})
			.inner;
		let changed = (self.kind, self.date) != previous;
		if changed {
			self.cursor = None;
		}
		(remove, changed)
	}

	fn date_button(&mut self, ui: &mut egui::Ui, width: f32, today: Date) {
		let label = crate::local_time::short_date(self.date);
		let colors = design::palette(ui);
		let height = design::SELECT_HEIGHT;
		let (rect, response) =
			ui.allocate_exact_size(egui::vec2(width.max(1.0), height), egui::Sense::click());
		let popup_id = egui::Popup::default_response_id(&response);
		response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &label));
		if !ui.is_rect_visible(rect) {
			egui::Popup::close_id(ui.ctx(), popup_id);
			return;
		}
		let active = response.has_focus() || egui::Popup::is_id_open(ui.ctx(), popup_id);
		let (fill, border) = if active {
			(colors.raised, colors.accent)
		} else if response.hovered() {
			(colors.hover, colors.muted)
		} else {
			(colors.base, colors.border)
		};
		ui.painter().rect(
			rect,
			8,
			fill,
			egui::Stroke::new(1.0, border),
			egui::StrokeKind::Inside,
		);
		let glyph = egui::Rect::from_center_size(
			rect.right_center() - egui::vec2(17.0, 0.0),
			egui::Vec2::splat(16.0),
		);
		icons::paint(ui.painter(), icons::Icon::Calendar, glyph, colors.muted);
		let galley = egui::WidgetText::from(label).into_galley(
			ui,
			Some(egui::TextWrapMode::Truncate),
			(glyph.left() - rect.left() - 14.0).max(0.0),
			egui::TextStyle::Body,
		);
		ui.painter().galley(
			egui::pos2(rect.left() + 10.0, rect.center().y - galley.size().y / 2.0),
			galley,
			colors.text_strong,
		);
		if response.clicked() {
			self.month = month_start(self.date.clamp(earliest(), today));
		}
		let selected = egui::Popup::from_toggle_button_response(&response)
			.close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
			.align(egui::RectAlign::TOP_START)
			.gap(6.0)
			.frame(
				egui::Frame::popup(ui.style())
					.fill(colors.raised.to_opaque())
					.stroke(egui::Stroke::new(1.0, colors.border))
					.corner_radius(12)
					.inner_margin(10),
			)
			.show(|ui| self.calendar(ui, today))
			.and_then(|popup| popup.inner);
		if let Some(date) = selected {
			self.date = date;
			response.request_focus();
		}
	}

	fn calendar(&mut self, ui: &mut egui::Ui, today: Date) -> Option<Date> {
		let width = (ui.ctx().content_rect().width() - 36.0).clamp(140.0, 260.0);
		ui.set_width(width);
		ui.spacing_mut().item_spacing = egui::vec2(3.0, 3.0);
		let cell_width = (width - 6.0 * ui.spacing().item_spacing.x) / 7.0;
		let colors = design::palette(ui);
		let month = self.month;
		let earliest = earliest();
		let title = translate_count(
			"search-date-month-year",
			usize::from(u8::from(month.month())),
			&[("year", &month.year().to_string())],
		);
		ui.horizontal(|ui| {
			let previous = ui.add_enabled_ui(month > month_start(earliest), |ui| {
				icons::button(
					ui,
					icons::Icon::CaretLeft,
					26.0,
					"search-date-previous-month",
				)
			});
			if previous.inner.clicked() {
				self.month = month_start(month - Duration::DAY);
			}
			ui.add_sized(
				[width - 58.0, 26.0],
				egui::Label::new(design::semibold(ui, title, 14.0)).truncate(),
			);
			let next = ui.add_enabled_ui(month < month_start(today), |ui| {
				icons::button(
					ui,
					icons::Icon::ChevronRight,
					26.0,
					"search-date-next-month",
				)
			});
			if next.inner.clicked() {
				self.month = month_start(month + Duration::days(32));
			}
		});
		ui.horizontal(|ui| {
			for weekday in 0..7 {
				let label = translate_count("search-date-weekday", weekday, &[]);
				ui.add_sized(
					[cell_width, 22.0],
					egui::Label::new(egui::RichText::new(label).size(11.0).color(colors.muted)),
				);
			}
		});
		let (start, weeks) = calendar_weeks(month);
		let mut selected = None;
		for week in 0..i64::from(weeks) {
			ui.horizontal(|ui| {
				for weekday in 0..7 {
					let date = start + Duration::days(week * 7 + weekday);
					let available =
						date.month() == month.month() && (earliest..=today).contains(&date);
					let response = ui
						.add_enabled_ui(available, |ui| {
							day_cell(ui, cell_width, date, date == self.date)
						})
						.inner;
					if response.clicked() {
						selected = Some(date);
						ui.close();
					}
				}
			});
		}
		selected
	}
}

fn day_cell(ui: &mut egui::Ui, width: f32, date: Date, selected: bool) -> egui::Response {
	let colors = design::palette(ui);
	let (rect, response) = ui.allocate_exact_size(egui::vec2(width, 28.0), egui::Sense::click());
	if selected {
		ui.painter()
			.rect_filled(rect, 6, colors.accent.gamma_multiply(0.35));
	} else if response.hovered() {
		ui.painter().rect_filled(rect, 6, colors.hover);
	}
	if response.has_focus() {
		ui.painter().rect_stroke(
			rect,
			6,
			egui::Stroke::new(1.0, colors.accent),
			egui::StrokeKind::Inside,
		);
	}
	ui.painter().text(
		rect.center(),
		egui::Align2::CENTER_CENTER,
		date.day().to_string(),
		egui::TextStyle::Body.resolve(ui.style()),
		if ui.is_enabled() {
			colors.text_strong
		} else {
			colors.muted
		},
	);
	response.widget_info(|| {
		egui::WidgetInfo::selected(
			egui::Role::Button,
			ui.is_enabled(),
			selected,
			crate::local_time::short_date(date),
		)
	});
	response
}

fn month_start(date: Date) -> Date {
	date - Duration::days(i64::from(date.day()) - 1)
}

fn calendar_weeks(month: Date) -> (Date, u8) {
	let leading_days = month.weekday().number_days_from_sunday();
	let start = month - Duration::days(i64::from(leading_days));
	let weeks = (leading_days + month.month().length(month.year())).div_ceil(7);
	(start, weeks)
}

#[derive(Default)]
pub(super) struct Dates(Vec<Row>);

impl Dates {
	pub(super) fn from_query(query: &str) -> Self {
		if let Some(date) = during_date(query) {
			return Self(vec![Row::new(Kind::During, date)]);
		}
		let (after, before) = effective_bounds(query);
		Self(
			[(Kind::Before, before), (Kind::After, after)]
				.into_iter()
				.filter_map(|(kind, cursor)| {
					let mut row = Row::new(kind, date_at(cursor?)?);
					row.cursor = cursor;
					Some(row)
				})
				.collect(),
		)
	}

	pub(super) fn show(&mut self, ui: &mut egui::Ui) -> bool {
		let today = OffsetDateTime::now_utc().date().max(earliest());
		let only_kind = self.0.len() == 2;
		let mut changed = false;
		let mut removed = None;
		for (index, row) in self.0.iter_mut().enumerate() {
			let (remove, edited) = ui.push_id(index, |ui| row.show(ui, today, only_kind)).inner;
			if remove {
				removed = Some(index);
			}
			changed |= edited;
		}
		if let Some(index) = removed {
			self.0.remove(index);
			changed = true;
		}
		let next = match self.0.as_slice() {
			[] => Some((Kind::Before, today)),
			[row] if row.kind == Kind::Before => Some((
				Kind::After,
				(row.date - Duration::DAY).clamp(earliest(), today),
			)),
			[row] if row.kind == Kind::After => Some((
				Kind::Before,
				(row.date + Duration::DAY).clamp(earliest(), today),
			)),
			_ => None,
		};
		if let Some((kind, date)) = next
			&& ui
				.add_sized(
					[ui.available_width(), design::SELECT_HEIGHT],
					egui::Button::new(translate("search-filters-show-add-date")),
				)
				.clicked()
		{
			self.0.push(Row::new(kind, date));
			changed = true;
		}
		changed
	}

	pub(super) fn bounds(&self) -> Result<[(&'static str, Option<u64>); 2], &'static str> {
		let mut after = None;
		let mut before = None;
		const INVALID: &str = "search-date-invalid";
		for row in &self.0 {
			match row.kind {
				Kind::Before => {
					before = Some(row.cursor.or_else(|| day_start(row.date)).ok_or(INVALID)?)
				}
				Kind::After => {
					after = Some(row.cursor.or_else(|| day_start(row.date)).ok_or(INVALID)?)
				}
				Kind::During => {
					let (start, end) = during_bounds(row.date).ok_or(INVALID)?;
					after = Some(start);
					before = Some(end);
				}
			}
		}
		if after.zip(before).is_some_and(|(start, end)| start >= end) {
			return Err("search-date-invalid-range");
		}
		Ok([("after_id", after), ("before_id", before)])
	}
}

#[cfg(test)]
mod tests {
	#[test]
	fn calendar_shows_only_weeks_that_contain_days_in_the_month() {
		for (month, weeks) in [("2026-02-01", 4), ("2026-11-01", 5), ("2026-08-01", 6)] {
			let month = super::parse_date(month).unwrap();
			let (start, actual_weeks) = super::calendar_weeks(month);
			assert_eq!(start.weekday(), time::Weekday::Sunday);
			assert_eq!(actual_weeks, weeks);
		}
	}
}
