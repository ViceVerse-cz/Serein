//! Device-local clock style for message times; independent of Discord accounts.

/// How times of day are written. `Auto` follows the display language, like Discord.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TimeFormat {
	#[default]
	Auto,
	TwelveHour,
	TwentyFourHour,
}
impl TimeFormat {
	pub const ALL: [Self; 3] = [Self::Auto, Self::TwelveHour, Self::TwentyFourHour];
	/// Stable storage key; unknown keys from a newer build fall back to [`Self::Auto`].
	fn key(self) -> &'static str {
		match self {
			Self::Auto => "auto",
			Self::TwelveHour => "12-hour",
			Self::TwentyFourHour => "24-hour",
		}
	}
}
impl serde::Serialize for TimeFormat {
	fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
		serializer.serialize_str(self.key())
	}
}
impl<'de> serde::Deserialize<'de> for TimeFormat {
	fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
		let value = <String as serde::Deserialize>::deserialize(deserializer)?;
		Ok(Self::ALL
			.into_iter()
			.find(|candidate| candidate.key() == value)
			.unwrap_or_default())
	}
}
