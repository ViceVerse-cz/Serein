//! Leading composer syntax and bounded admission shared by UI, state and transport.
/// Seven marker characters plus one separating whitespace scalar.
pub const PREFIX_ALLOWANCE: usize = 8;

pub fn content(value: &str) -> (&str, bool) {
	match value.strip_prefix("@silent") {
		Some(rest) if rest.is_empty() || rest.starts_with(char::is_whitespace) => {
			let separator = rest.chars().next().map_or(0, char::len_utf8);
			(&rest[separator..], true)
		}
		_ => (value, false),
	}
}

/// Forum editors trim ordinary starters; a quiet starter retains formatting after its marker.
pub fn starter(value: &str) -> &str {
	let leading_trimmed = value.trim_start();
	if content(leading_trimmed).1 {
		leading_trimmed
	} else {
		value.trim()
	}
}

pub fn valid(value: &str, maximum: usize, allow_empty: bool) -> bool {
	if value.len() > maximum.saturating_add(PREFIX_ALLOWANCE).saturating_mul(4) {
		return false;
	}
	let (effective, silent) = content(value);
	value.chars().count() <= maximum.saturating_add(if silent { PREFIX_ALLOWANCE } else { 0 })
		&& effective.chars().count() <= maximum
		&& (allow_empty || !effective.trim().is_empty())
}

#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn quiet_admission_counts_effective_text_and_bounds_marker_overhead() {
		assert!(!valid("@silent", 2000, false));
		assert!(valid("@silent", 2000, true));
		assert!(valid(&format!("@silent {}", "é".repeat(2000)), 2000, false));
		assert!(!valid(
			&format!("@silent {}", "x".repeat(2001)),
			2000,
			false
		));
		assert!(!valid(
			&format!("@silent{}x", " ".repeat(2008)),
			2000,
			false
		));
		assert!(!valid(&"x".repeat(2008), 2000, false));
		assert_eq!(content("@silently hi"), ("@silently hi", false));
		assert_eq!(
			content("@silent\n    indented\n  "),
			("    indented\n  ", true)
		);
		assert_eq!(content("@silent  padded"), (" padded", true));
		assert_eq!(starter("  @silent\n    code\n  "), "@silent\n    code\n  ");
		assert_eq!(starter("  ordinary  "), "ordinary");
	}
}
