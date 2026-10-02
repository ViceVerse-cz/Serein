//! UI-neutral limits and outcomes for explicitly consented anonymous Catbox uploads.
pub const MAX_BYTES: u64 = 200_000_000;

pub fn eligible(filename: &str, bytes: u64) -> bool {
	let extension = filename
		.rsplit_once('.')
		.map_or("", |(_, ext)| ext)
		.to_ascii_lowercase();
	bytes > 0
		&& bytes <= MAX_BYTES
		&& !matches!(extension.as_str(), "exe" | "scr" | "cpl" | "jar")
		&& !extension.starts_with("doc")
		&& !(extension == "gif" && bytes > 20_000_000)
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
