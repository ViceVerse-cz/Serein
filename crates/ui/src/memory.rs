//! Memory Serein can account for itself: egui textures by name, the glyph atlas, font files and
//! each image working set's own byte count. Allocator, driver and swapchain memory are not
//! visible here; process totals come from OS tools.
use crate::MessagingUi;
use std::{
	collections::{BTreeMap, HashSet},
	fmt::Write,
	sync::Arc,
};

/// Texture names listed one per line; smaller groups are summed into one line.
const NAMED_GROUPS: usize = 12;

/// Texture names Serein creates without per-item data.
const NAMES: [&str; 22] = [
	"egui_font_texture",
	"service-image",
	"service-animation",
	"fitted-image",
	"inline-video",
	"inline-video-shade",
	"inline-video-poster",
	"remote-stream",
	"local-screen",
	"local-camera",
	"settings-camera",
	"synthetic-stream",
	"emoji-blank",
	"theme-background",
	"theme-cover-thumbnail",
	"theme-image-thumbnail",
	"role-icon-draft",
	"server-icon-draft",
	"server-sticker-upload",
	"profile-avatar-preview",
	"create-server-icon",
	"group-icon-preview",
];

/// Names that carry a version, index, user or package after this prefix.
const PREFIXES: [&str; 6] = [
	"Twemoji ",
	"Phosphor Icons ",
	"attachment-preview-",
	"extension-preview-",
	"server-emoji-upload-",
	"remote-camera-",
];

fn mib(bytes: usize) -> f64 {
	bytes as f64 / (1024.0 * 1024.0)
}

/// The report's label for a texture. Any other name, such as one an image loader takes from
/// a URI, is counted as "other" so no identifier reaches a report.
fn label(name: &str) -> &'static str {
	if let Some(known) = NAMES.iter().find(|known| **known == name) {
		return known;
	}
	PREFIXES
		.iter()
		.find(|prefix| name.starts_with(**prefix))
		.map_or("other", |prefix| prefix.trim_end_matches(['-', ' ']))
}

impl MessagingUi {
	/// A Markdown section for the diagnostics report and `SEREIN_MEMORY_DIAGNOSTICS`.
	pub fn memory_report(&self, ctx: &egui::Context) -> String {
		let mut groups: BTreeMap<&str, (usize, usize)> = BTreeMap::new();
		for (_, meta) in ctx.tex_manager().read().allocated() {
			let entry = groups.entry(label(&meta.name)).or_default();
			entry.0 += 1;
			entry.1 += meta.bytes_used();
		}
		let mut groups: Vec<_> = groups.into_iter().collect();
		groups.sort_by_key(|(_, (_, bytes))| std::cmp::Reverse(*bytes));
		let count: usize = groups.iter().map(|(_, (count, _))| count).sum();
		let total: usize = groups.iter().map(|(_, (_, bytes))| bytes).sum();
		let (atlas, font_bytes) = ctx.fonts(|fonts| {
			let mut seen = HashSet::new();
			let font_bytes = fonts
				.definitions()
				.font_data
				.values()
				.map(|data| &data.font)
				.chain(
					fonts
						.discovered_fonts()
						.iter()
						.map(|insert| &insert.data.font),
				)
				.filter(|blob| seen.insert(Arc::as_ptr(blob).cast::<()>()))
				.map(|blob| AsRef::<[u8]>::as_ref(&**blob).len())
				.sum::<usize>();
			(fonts.font_image_resolution(), font_bytes)
		});

		let mut report = String::from("## Memory\n\n");
		let _ = writeln!(report, "- **Textures:** {count}, {:.1} MiB", mib(total));
		for (name, (count, bytes)) in groups.iter().take(NAMED_GROUPS) {
			let _ = writeln!(report, "  - {name}: {count}, {:.1} MiB", mib(*bytes));
		}
		if let Some(rest) = groups.get(NAMED_GROUPS..).filter(|rest| !rest.is_empty()) {
			let count: usize = rest.iter().map(|(_, (count, _))| count).sum();
			let bytes: usize = rest.iter().map(|(_, (_, bytes))| bytes).sum();
			let _ = writeln!(
				report,
				"  - {} other names: {count}, {:.1} MiB",
				rest.len(),
				mib(bytes)
			);
		}
		// The atlas keeps a CPU image of the same size as its texture.
		let _ = writeln!(
			report,
			"- **Glyph atlas:** {}×{}, {:.1} MiB in RAM",
			atlas[0],
			atlas[1],
			mib(atlas[0] * atlas[1] * 4)
		);
		let _ = writeln!(report, "- **Font files:** {:.1} MiB", mib(font_bytes));
		report.push_str("- **Image working sets:**\n");
		for (name, count, bytes) in self.avatars.usage() {
			let _ = writeln!(report, "  - {name}: {count}, {:.1} MiB", mib(bytes));
		}
		report
	}
}

#[cfg(test)]
mod tests {
	/// Texture names that carry a package, user or URI never reach a report.
	#[test]
	fn texture_labels_never_carry_identifiers() {
		assert_eq!(
			super::label("extension-preview-theme-123456789012345678-private"),
			"extension-preview"
		);
		assert_eq!(super::label("https://cdn.example/private.png"), "other");
		assert_eq!(super::label("service-image"), "service-image");
	}
}
