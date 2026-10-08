//! UI-neutral limits and outcomes for explicitly consented anonymous public uploads.

/// Anonymous public file host. x0.at (a 0x0 instance with direct, embeddable links) is the
/// default; Catbox and its temporary 72-hour Litterbox service remain alternatives.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Host {
	#[default]
	X0At,
	Catbox,
	Litterbox,
}

impl Host {
	pub const ALL: [Self; 3] = [Self::X0At, Self::Catbox, Self::Litterbox];

	pub fn name(self) -> &'static str {
		match self {
			Self::X0At => "x0.at",
			Self::Catbox => "Catbox",
			Self::Litterbox => "Litterbox",
		}
	}

	pub fn max_bytes(self) -> u64 {
		match self {
			Self::X0At => 1024 * 1024 * 1024,
			Self::Catbox => 200_000_000,
			Self::Litterbox => 1_000_000_000,
		}
	}

	/// Every link the host returns starts with this prefix and names one flat file.
	pub fn link_prefix(self) -> &'static str {
		match self {
			Self::X0At => "https://x0.at/",
			Self::Catbox => "https://files.catbox.moe/",
			Self::Litterbox => "https://litter.catbox.moe/",
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
			Host::X0At => true,
			Host::Catbox => {
				!extension.starts_with("doc") && !(extension == "gif" && bytes > 20_000_000)
			}
			Host::Litterbox => !extension.starts_with("doc"),
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
	Unavailable,
	ConversationChanged,
	SelectionChanged,
	MissingSelection,
}
