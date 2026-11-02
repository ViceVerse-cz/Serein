//! Converts UTC instants to the user's local zone for display.
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};

/// Whether times of day use a 12-hour clock; set from Settings › Language & Time.
static TWELVE_HOUR: AtomicBool = AtomicBool::new(false);
/// The [`DateOrder`] of numeric dates; follows the display language or system locale.
static DATE_ORDER: AtomicU8 = AtomicU8::new(DateOrder::MonthDayYear as u8);

/// Applies the clock style and date order. `Auto` follows the display language, or the system
/// locale when the language follows the system, as Discord does.
pub fn set_format(format: model::TimeFormat, language: crate::i18n::Language) {
	let system = || sys_locale::get_locale().unwrap_or_else(|| "en-US".to_owned());
	let tag = language.preference().map_or_else(system, str::to_owned);
	let twelve = match format {
		model::TimeFormat::TwelveHour => true,
		model::TimeFormat::TwentyFourHour => false,
		model::TimeFormat::Auto => twelve_hour_locale(&tag),
	};
	TWELVE_HOUR.store(twelve, Ordering::Relaxed);
	DATE_ORDER.store(DateOrder::of_locale(&tag) as u8, Ordering::Relaxed);
}

/// How a locale writes a numeric date, as Discord's short date (`L`) does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DateOrder {
	/// `04/14/2025`
	MonthDayYear,
	/// `14/04/2025`
	DayMonthYear,
	/// `14.04.2025`
	DayMonthYearDots,
	/// `2025/04/14`
	YearMonthDay,
	/// `2025-04-14`
	YearMonthDayDashes,
}

impl DateOrder {
	const ALL: [Self; 5] = [
		Self::MonthDayYear,
		Self::DayMonthYear,
		Self::DayMonthYearDots,
		Self::YearMonthDay,
		Self::YearMonthDayDashes,
	];

	fn current() -> Self {
		let value = DATE_ORDER.load(Ordering::Relaxed);
		Self::ALL
			.into_iter()
			.find(|order| *order as u8 == value)
			.unwrap_or(Self::MonthDayYear)
	}

	fn of_locale(tag: &str) -> Self {
		let (language, region) = locale_parts(tag);
		// Regional overrides come before the language-wide orders below.
		match (language.as_str(), region.as_deref()) {
			("en", None | Some("US" | "PH")) | ("es", Some("US")) => Self::MonthDayYear,
			("en" | "fr", Some("CA")) | ("sv" | "lt", _) => Self::YearMonthDayDashes,
			("en", Some("ZA")) | ("ja" | "zh" | "ko" | "hu", _) => Self::YearMonthDay,
			(
				"de" | "ru" | "tr" | "pl" | "cs" | "sk" | "uk" | "fi" | "da" | "nb" | "no" | "ro"
				| "bg" | "hr" | "sr" | "lv" | "et",
				_,
			) => Self::DayMonthYearDots,
			_ => Self::DayMonthYear,
		}
	}

	fn date(self, date: time::Date) -> String {
		let (year, month, day) = (date.year(), u8::from(date.month()), date.day());
		match self {
			Self::MonthDayYear => format!("{month:02}/{day:02}/{year}"),
			Self::DayMonthYear => format!("{day:02}/{month:02}/{year}"),
			Self::DayMonthYearDots => format!("{day:02}.{month:02}.{year}"),
			Self::YearMonthDay => format!("{year}/{month:02}/{day:02}"),
			Self::YearMonthDayDashes => format!("{year}-{month:02}-{day:02}"),
		}
	}
}

/// A locale tag's lowercase language and uppercase two-letter region, ignoring encodings.
fn locale_parts(tag: &str) -> (String, Option<String>) {
	let tag = tag.split(['.', '@']).next().unwrap_or_default();
	let tag = tag.replace('_', "-");
	let mut parts = tag.split('-');
	let language = parts.next().unwrap_or_default().to_ascii_lowercase();
	// Single-letter subtags start extensions (`-u-ca-gregory`), whose keys are not regions.
	let region = parts
		.take_while(|part| part.len() > 1)
		.find(|part| part.len() == 2)
		.map(str::to_ascii_uppercase);
	(language, region)
}

/// Locales whose convention is a 12-hour clock with AM/PM.
fn twelve_hour_locale(tag: &str) -> bool {
	let (language, region) = locale_parts(tag);
	match (language.as_str(), region.as_deref()) {
		("en", None) => true,
		("en", Some(region)) => matches!(region, "US" | "CA" | "AU" | "NZ" | "PH" | "IN"),
		_ => false,
	}
}

/// A time of day in the chosen clock style: `14:05` or `2:05 PM`.
pub fn clock(at: time::OffsetDateTime) -> String {
	clock_with(at, false)
}

/// [`clock`] with seconds: `14:05:09` or `2:05:09 PM`.
pub fn clock_seconds(at: time::OffsetDateTime) -> String {
	clock_with(at, true)
}

fn clock_with(at: time::OffsetDateTime, seconds: bool) -> String {
	format_clock(at, seconds, TWELVE_HOUR.load(Ordering::Relaxed))
}

fn format_clock(at: time::OffsetDateTime, seconds: bool, twelve: bool) -> String {
	let seconds = if seconds {
		format!(":{:02}", at.second())
	} else {
		String::new()
	};
	if twelve {
		let hour = match at.hour() % 12 {
			0 => 12,
			hour => hour,
		};
		let suffix = if at.hour() < 12 { "AM" } else { "PM" };
		format!("{hour}:{:02}{seconds} {suffix}", at.minute())
	} else {
		format!("{:02}:{:02}{seconds}", at.hour(), at.minute())
	}
}

/// Shifts `instant` to the local offset in effect at that moment; falls back to UTC.
pub fn local(instant: time::OffsetDateTime) -> time::OffsetDateTime {
	// Tests pin UTC so date-boundary assertions hold on every machine.
	let offset = if cfg!(test) {
		time::UtcOffset::UTC
	} else {
		time::UtcOffset::local_offset_at(instant).unwrap_or(time::UtcOffset::UTC)
	};
	instant.to_offset(offset)
}

/// Current wall-clock time in the local zone.
pub fn now() -> time::OffsetDateTime {
	local(time::OffsetDateTime::now_utc())
}

/// Renders a Discord `<t:seconds:style>` reference the way the official client does: absolute
/// styles in the viewer's zone, `R` as a coarse relative phrase that refreshes on every frame.
pub fn discord_timestamp(seconds: i64, style: u8) -> Option<String> {
	let instant = time::OffsetDateTime::from_unix_timestamp(seconds).ok()?;
	if style == b'R' {
		return Some(relative(instant - time::OffsetDateTime::now_utc()));
	}
	let at = local(instant);
	let date = at.date();
	let clock = clock(at);
	Some(match style {
		b't' => clock,
		b'T' => clock_seconds(at),
		b'd' => format!(
			"{:02}/{:02}/{}",
			u8::from(date.month()),
			date.day(),
			date.year()
		),
		b'D' => format!("{} {}, {}", date.month(), date.day(), date.year()),
		b'F' => format!(
			"{}, {} {}, {} {clock}",
			date.weekday(),
			date.month(),
			date.day(),
			date.year()
		),
		_ => format!("{} {}, {} {clock}", date.month(), date.day(), date.year()),
	})
}
/// A message header's time as Discord writes it: just the time today, "Yesterday at 11:22", and
/// older days as the locale's numeric date and time, e.g. `14/04/2025 11:22`.
pub fn header(utc: time::OffsetDateTime) -> String {
	format_header(
		local(utc),
		now().date(),
		TWELVE_HOUR.load(Ordering::Relaxed),
		DateOrder::current(),
	)
}

/// [`header`] against an explicit local `today`, so day boundaries are testable.
fn format_header(
	at: time::OffsetDateTime,
	today: time::Date,
	twelve: bool,
	order: DateOrder,
) -> String {
	let clock = format_clock(at, false, twelve);
	if at.date() == today {
		clock
	} else if today.previous_day() == Some(at.date()) {
		crate::i18n::translate_args("search-result-yesterday-at", &[("time", &clock)])
	} else {
		format!("{} {clock}", order.date(at.date()))
	}
}

/// The full date shown when hovering a message time: "Monday, April 14, 2025 at 11:22 AM",
/// "Monday, 14 April 2025 at 11:22" where the locale writes the day first, or
/// "Monday, 2025 April 14 at 11:22" where it writes the year first.
pub fn full(utc: time::OffsetDateTime) -> String {
	format_full(
		local(utc),
		TWELVE_HOUR.load(Ordering::Relaxed),
		DateOrder::current(),
	)
}

fn format_full(at: time::OffsetDateTime, twelve: bool, order: DateOrder) -> String {
	let date = at.date();
	let clock = format_clock(at, false, twelve);
	match order {
		DateOrder::MonthDayYear => format!(
			"{}, {} {}, {} at {clock}",
			date.weekday(),
			date.month(),
			date.day(),
			date.year()
		),
		DateOrder::YearMonthDay | DateOrder::YearMonthDayDashes => format!(
			"{}, {} {} {} at {clock}",
			date.weekday(),
			date.year(),
			date.month(),
			date.day()
		),
		DateOrder::DayMonthYear | DateOrder::DayMonthYearDots => format!(
			"{}, {} {} {} at {clock}",
			date.weekday(),
			date.day(),
			date.month(),
			date.year()
		),
	}
}

/// Discord's calendar time: "Today at 2:05 PM", "Yesterday at 14:05", else "10/12/26, 5:37 PM".
pub fn calendar(utc: time::OffsetDateTime) -> String {
	let at = local(utc);
	let clock = clock(at);
	let today = now().date();
	if at.date() == today {
		crate::i18n::translate_args("search-result-today-at", &[("time", &clock)])
	} else if today.previous_day() == Some(at.date()) {
		crate::i18n::translate_args("search-result-yesterday-at", &[("time", &clock)])
	} else {
		// Discord writes older times as "10/12/26, 5:37 PM".
		let date = at.date();
		format!(
			"{}/{}/{:02}, {clock}",
			u8::from(date.month()),
			date.day(),
			date.year().rem_euclid(100)
		)
	}
}
/// "… ago" for a past instant, measured against the current clock.
pub(crate) fn ago(instant: time::OffsetDateTime) -> String {
	relative(instant - time::OffsetDateTime::now_utc())
}
/// Coarse "in …"/"… ago" phrasing with the same thresholds the web client's relative times use.
fn relative(delta: time::Duration) -> String {
	let seconds = delta.whole_seconds();
	let ahead = seconds > 0;
	let seconds = seconds.unsigned_abs();
	// Rounded like the web client: 47 hours reads "2 days", not "1 day".
	let minutes = (seconds + 30) / 60;
	let hours = (minutes + 30) / 60;
	let days = (hours + 12) / 24;
	let months = (days + 15) / 30;
	let years = (days + 182) / 365;
	let amount = if seconds < 45 {
		"a few seconds".to_owned()
	} else if seconds < 90 {
		"a minute".to_owned()
	} else if minutes < 45 {
		format!("{minutes} minutes")
	} else if minutes < 90 {
		"an hour".to_owned()
	} else if hours < 22 {
		format!("{hours} hours")
	} else if hours < 36 {
		"a day".to_owned()
	} else if days < 26 {
		format!("{days} days")
	} else if days < 46 {
		"a month".to_owned()
	} else if days < 320 {
		format!("{months} months")
	} else if days < 548 {
		"a year".to_owned()
	} else {
		format!("{years} years")
	};
	if ahead {
		format!("in {amount}")
	} else {
		format!("{amount} ago")
	}
}

#[cfg(test)]
mod tests {
	#[test]
	fn clocks_follow_the_twelve_or_twenty_four_hour_style() {
		let at = |hour, minute| {
			time::OffsetDateTime::UNIX_EPOCH
				.replace_time(time::Time::from_hms(hour, minute, 9).unwrap())
		};
		for (hour, twelve, twenty_four) in [
			(0, "12:05 AM", "00:05"),
			(9, "9:05 AM", "09:05"),
			(12, "12:05 PM", "12:05"),
			(23, "11:05 PM", "23:05"),
		] {
			assert_eq!(super::format_clock(at(hour, 5), false, true), twelve);
			assert_eq!(super::format_clock(at(hour, 5), false, false), twenty_four);
		}
		assert_eq!(super::format_clock(at(14, 5), true, true), "2:05:09 PM");
		assert_eq!(super::format_clock(at(14, 5), true, false), "14:05:09");
	}

	/// Drops Fluent's bidi isolation marks around placeables.
	fn plain(text: String) -> String {
		text.replace(['\u{2068}', '\u{2069}'], "")
	}

	fn local_at(rfc3339: &str) -> time::OffsetDateTime {
		time::OffsetDateTime::parse(rfc3339, &time::format_description::well_known::Rfc3339)
			.unwrap()
	}

	/// Matches the issue's Discord reference: `14/04/2025 11:22` for a day-first locale.
	#[test]
	fn headers_show_the_time_today_yesterday_and_a_locale_date_before() {
		use super::DateOrder::*;
		let today = time::Date::from_calendar_date(2026, time::Month::October, 14).unwrap();
		let header =
			|at, twelve, order| plain(super::format_header(local_at(at), today, twelve, order));
		assert_eq!(
			header("2026-10-14T15:45:00+02:00", true, MonthDayYear),
			"3:45 PM"
		);
		assert_eq!(
			header("2026-10-14T15:45:00+02:00", false, DayMonthYear),
			"15:45"
		);
		assert_eq!(
			header("2026-10-13T15:45:00+02:00", true, MonthDayYear),
			"Yesterday at 3:45 PM"
		);
		assert_eq!(
			header("2026-10-13T15:45:00+02:00", false, DayMonthYearDots),
			"Yesterday at 15:45"
		);
		// Older days in the same year and earlier years both carry the full date.
		assert_eq!(
			header("2026-10-12T15:45:00+02:00", true, MonthDayYear),
			"10/12/2026 3:45 PM"
		);
		assert_eq!(
			header("2025-04-14T11:22:00+01:00", false, DayMonthYear),
			"14/04/2025 11:22"
		);
		assert_eq!(
			header("2025-04-14T11:22:00+01:00", false, DayMonthYearDots),
			"14.04.2025 11:22"
		);
		assert_eq!(
			header("2025-04-14T09:05:00+09:00", false, YearMonthDay),
			"2025/04/14 09:05"
		);
		assert_eq!(
			header("2025-04-14T09:05:00-05:00", true, YearMonthDayDashes),
			"2025-04-14 9:05 AM"
		);
	}

	/// Labels follow the local calendar day, not 24-hour windows from now.
	#[test]
	fn header_days_change_at_local_midnight() {
		let today = time::Date::from_calendar_date(2026, time::Month::October, 14).unwrap();
		let header = |at| {
			plain(super::format_header(
				local_at(at),
				today,
				false,
				super::DateOrder::DayMonthYear,
			))
		};
		assert_eq!(header("2026-10-14T00:00:00+02:00"), "00:00");
		assert_eq!(header("2026-10-14T23:59:00+02:00"), "23:59");
		assert_eq!(header("2026-10-13T23:59:00+02:00"), "Yesterday at 23:59");
		assert_eq!(header("2026-10-13T00:00:00+02:00"), "Yesterday at 00:00");
		assert_eq!(header("2026-10-12T23:59:00+02:00"), "12/10/2026 23:59");
		// Year and month boundaries still count as yesterday.
		let new_year = time::Date::from_calendar_date(2026, time::Month::January, 1).unwrap();
		assert_eq!(
			plain(super::format_header(
				local_at("2025-12-31T23:30:00Z"),
				new_year,
				true,
				super::DateOrder::MonthDayYear
			)),
			"Yesterday at 11:30 PM"
		);
		// The instant's own offset decides its day: 23:30 UTC is already tomorrow in Prague.
		let utc = local_at("2026-10-13T23:30:00Z");
		assert_eq!(
			super::format_header(
				utc.to_offset(time::UtcOffset::from_hms(2, 0, 0).unwrap()),
				today,
				false,
				super::DateOrder::DayMonthYear
			),
			"01:30"
		);
	}

	#[test]
	fn date_order_follows_the_locale() {
		use super::DateOrder::*;
		for (tag, order) in [
			("en-US", MonthDayYear),
			("en", MonthDayYear),
			("en_US.UTF-8", MonthDayYear),
			("en-GB", DayMonthYear),
			("en_GB.UTF-8@euro", DayMonthYear),
			("en-AU", DayMonthYear),
			("en-CA", YearMonthDayDashes),
			("en-IN", DayMonthYear),
			("en-NZ", DayMonthYear),
			("fr_CA.UTF-8", YearMonthDayDashes),
			("en-ZA", YearMonthDay),
			("en_ZA.UTF-8", YearMonthDay),
			("en-u-ca-gregory", MonthDayYear),
			("en-GB-u-ca-gregory", DayMonthYear),
			("fr-FR", DayMonthYear),
			("es-US", MonthDayYear),
			("es-ES", DayMonthYear),
			("zh-HK", YearMonthDay),
			("zh_CN.UTF-8", YearMonthDay),
			("ko-KR", YearMonthDay),
			("fr", DayMonthYear),
			("pt-BR", DayMonthYear),
			("de-DE", DayMonthYearDots),
			("cs_CZ.UTF-8", DayMonthYearDots),
			("ja-JP", YearMonthDay),
			("zh-TW", YearMonthDay),
			("sv-SE", YearMonthDayDashes),
		] {
			assert_eq!(super::DateOrder::of_locale(tag), order, "{tag}");
		}
		for order in super::DateOrder::ALL {
			assert_eq!(super::DateOrder::ALL[order as usize], order);
		}
	}

	#[test]
	fn auto_uses_twelve_hours_only_where_the_locale_does() {
		for tag in [
			"en-US",
			"en_US.UTF-8",
			"en_US@calendar=gregorian",
			"en",
			"en-CA",
			"en-AU",
		] {
			assert!(super::twelve_hour_locale(tag), "{tag}");
		}
		for tag in [
			"en-GB",
			"en_GB.UTF-8",
			"en_GB@calendar=gregorian",
			"en_GB.UTF-8@calendar=gregorian",
			"en_GB@euro",
			"de-DE",
			"fr",
			"ja-JP",
			"cs_CZ",
		] {
			assert!(!super::twelve_hour_locale(tag), "{tag}");
		}
	}

	#[test]
	fn formats_every_discord_timestamp_style() {
		let at = 1_700_000_000;
		for (style, expected) in [
			(b't', "22:13"),
			(b'T', "22:13:20"),
			(b'd', "11/14/2023"),
			(b'D', "November 14, 2023"),
			(b'f', "November 14, 2023 22:13"),
			(b'F', "Tuesday, November 14, 2023 22:13"),
		] {
			assert_eq!(
				super::discord_timestamp(at, style).as_deref(),
				Some(expected)
			);
		}
		assert_eq!(super::discord_timestamp(i64::MAX, b'f'), None);
	}
}
