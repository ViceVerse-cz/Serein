//! UI-neutral limits and outcomes for explicitly consented anonymous public uploads.

/// Anonymous public file host. 0x0.st is the default; Catbox stays available as an alternative.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Host {
	#[default]
	ZeroX0,
	Catbox,
}

impl Host {
	pub const ALL: [Self; 2] = [Self::ZeroX0, Self::Catbox];

	pub fn name(self) -> &'static str {
		match self {
			Self::ZeroX0 => "0x0.st",
			Self::Catbox => "Catbox",
		}
	}

	pub fn max_bytes(self) -> u64 {
		match self {
			Self::ZeroX0 => 512 * 1024 * 1024,
			Self::Catbox => 200_000_000,
		}
	}

	/// Every link the host returns starts with this prefix and names one flat file.
	pub fn link_prefix(self) -> &'static str {
		match self {
			Self::ZeroX0 => "https://0x0.st/",
			Self::Catbox => "https://files.catbox.moe/",
		}
	}
}

pub fn eligible(host: Host, filename: &str, bytes: u64) -> bool {
	let extension = filename
		.rsplit_once('.')
		.map_or("", |(_, ext)| ext)
		.to_ascii_lowercase();
	bytes > 0
		&& bytes <= host.max_bytes()
		&& !matches!(extension.as_str(), "exe" | "scr" | "cpl" | "jar" | "apk")
		&& match host {
			Host::ZeroX0 => true,
			Host::Catbox => {
				!extension.starts_with("doc") && !(extension == "gif" && bytes > 20_000_000)
			}
		}
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
	Cancelled,
	Prepare,
	Changed,
	Unsupported,
	Failed,
	Rejected,
	Incomplete,
	ResponseLimit,
	Interrupted,
	InvalidLink,
	Busy,
	ConversationChanged,
	SelectionChanged,
	MissingSelection,
}
