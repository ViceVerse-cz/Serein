//! Converts UTC instants to the user's local zone for display.
use std::sync::atomic::{AtomicBool, Ordering};

/// Whether times of day use a 12-hour clock; set from Settings › Language & Time.
static TWELVE_HOUR: AtomicBool = AtomicBool::new(false);

/// Applies the clock style. `Auto` follows the display language, or the system locale when the
/// language follows the system, as Discord does.
pub fn set_format(format: model::TimeFormat, language: crate::i18n::Language) {
	let twelve = match format {
		model::TimeFormat::TwelveHour => true,
		model::TimeFormat::TwentyFourHour => false,
		model::TimeFormat::Auto => match language {
			crate::i18n::Language::System => {
				twelve_hour_locale(sys_locale::get_locale().as_deref().unwrap_or("en-US"))
			}
			language => language == crate::i18n::Language::English,
		},
	};
	TWELVE_HOUR.store(twelve, Ordering::Relaxed);
}

/// Locales whose convention is a 12-hour clock with AM/PM.
fn twelve_hour_locale(tag: &str) -> bool {
	let tag = tag.split(['.', '@']).next().unwrap_or_default();
	let tag = tag.replace('_', "-");
	let mut parts = tag.split('-');
	let language = parts.next().unwrap_or_default().to_ascii_lowercase();
	let region = parts
		.find(|part| part.len() == 2)
		.map(str::to_ascii_uppercase);
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
	#[cfg(test)]
	if let Some(now) = TEST_NOW.get() {
		return now;
	}
	local(time::OffsetDateTime::now_utc())
}

#[cfg(test)]
thread_local! {
	static TEST_NOW: std::cell::Cell<Option<time::OffsetDateTime>> = const { std::cell::Cell::new(None) };
}

/// Pins this test thread's clock until the returned guard is dropped, including on panic.
#[cfg(test)]
pub(crate) fn pin_now(now: time::OffsetDateTime) -> impl Drop {
	struct Reset(Option<time::OffsetDateTime>);
	impl Drop for Reset {
		fn drop(&mut self) {
			TEST_NOW.set(self.0);
		}
	}
	Reset(TEST_NOW.replace(Some(now)))
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
	fn calendar_uses_the_pinned_local_date_for_today_yesterday_and_older_times() {
		let at = time::OffsetDateTime::parse(
			"2026-10-12T17:37:03Z",
			&time::format_description::well_known::Rfc3339,
		)
		.unwrap();
		let clock = super::clock(at);
		for (days, key) in [
			(0, "search-result-today-at"),
			(1, "search-result-yesterday-at"),
		] {
			let _clock = super::pin_now(at + time::Duration::days(days));
			assert_eq!(
				super::calendar(at),
				crate::i18n::translate_args(key, &[("time", &clock)])
			);
		}
		let _clock = super::pin_now(at + time::Duration::days(2));
		assert_eq!(super::calendar(at), format!("10/12/26, {clock}"));
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
