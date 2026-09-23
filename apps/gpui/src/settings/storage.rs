//! Data & Privacy: what this frontend keeps on the device, with the two things it can clear:
//! the in-memory image cache and the signed-in account's saved drafts in its own store.
//! The main app's store is never opened here.
use super::kit;
use crate::{Serein, images};
use gpui::{prelude::*, *};

impl Serein {
	pub(super) fn settings_storage(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> Div {
		let demo = self.state.demo;
		let usage = images::usage();
		let images_detail = match usage {
			Some((count, bytes)) => format!(
				"Avatars, server icons and previews held in memory: {count} {} · {} of {}. They download again when shown.",
				if count == 1 { "image" } else { "images" },
				megabytes(bytes),
				megabytes(images::MAX_BYTES),
			),
			None => "Avatars, server icons and previews stay in memory only and download again when shown. The offline preview loads none.".to_owned(),
		};
		let has_images = usage.is_some_and(|(count, _)| count > 0);
		let drafts = self.saved_draft_count(cx);
		let can_clear = !demo && self.can_clear_drafts();
		let drafts_detail = if demo {
			"Unsent messages you left in conversations. The offline preview keeps none on disk."
				.to_owned()
		} else {
			format!(
				"Unsent messages this preview keeps for the signed-in account ({drafts} {}). Your login, settings and collapsed categories stay.",
				if drafts == 1 { "draft" } else { "drafts" }
			)
		};
		div()
			.flex()
			.flex_col()
			.gap_3()
			.child(kit::group(
				"Local storage",
				kit::card()
					.child(kit::row(
						"Clear image cache",
						Some(&images_detail),
						kit::button(
							"clear-image-cache",
							"Clear cache",
							kit::ButtonKind::Outline,
							has_images,
						)
						.when(has_images, |d| {
							d.on_click(cx.listener(|this, _, window, cx| {
								let count = images::clear(window, cx);
								this.notify_user(match count {
									1 => "Cleared 1 cached image.".to_owned(),
									count => format!("Cleared {count} cached images."),
								});
								cx.notify();
							}))
						}),
					))
					.child(kit::divider())
					.child(kit::row(
						"Clear saved drafts",
						Some(&drafts_detail),
						kit::button(
							"clear-drafts",
							"Clear drafts",
							kit::ButtonKind::Outline,
							can_clear,
						)
						.when(can_clear, |d| {
							d.on_click(
								cx.listener(|this, _, window, cx| this.confirm_clear_drafts(window, cx)),
							)
						}),
					))
					.child(kit::divider())
					.child(kit::hint(
						"This preview keeps drafts, collapsed categories and appearance choices in its own bounded SQLite file in the serein-gpui app data folder, with the window size beside it, never in the main Serein app's data. Messages are not cached on disk and images stay in memory. The file is not encrypted by Serein; the saved login token stays in the OS credential store.",
					)),
			))
			.child(kit::group(
				"Your privacy",
				kit::card().child(kit::hint(
					"Serein does not collect telemetry or upload diagnostics. Discord retains service-side data according to its own policies.",
				)),
			))
	}

	/// Drafts kept for the account: stored ones for other conversations plus the open composer.
	fn saved_draft_count(&self, cx: &App) -> usize {
		let selected = self.state.selected;
		let others = self
			.state
			.drafts
			.keys()
			.filter(|channel| Some(**channel) != selected)
			.count();
		let open = selected.is_some() && !self.composer.read(cx).value().is_empty();
		others + usize::from(open)
	}

	fn confirm_clear_drafts(&mut self, window: &mut Window, cx: &mut Context<Self>) {
		let answer = window.prompt(
			PromptLevel::Warning,
			"Clear saved drafts?",
			Some(
				"Unsent messages in every conversation of this account are deleted from this preview, including the one you are writing. The main Serein app's drafts are not affected.",
			),
			&["Clear drafts", "Cancel"],
			cx,
		);
		cx.spawn(async move |this, cx| {
			if answer.await == Ok(0) {
				let _ = this.update(cx, |this, cx| {
					this.clear_saved_drafts(cx);
					cx.notify();
				});
			}
		})
		.detach();
	}
}

fn megabytes(bytes: usize) -> String {
	let value = bytes as f64 / (1024. * 1024.);
	if value < 10. && bytes > 0 {
		format!("{value:.1} MB")
	} else {
		format!("{value:.0} MB")
	}
}

#[cfg(test)]
mod tests {
	use super::megabytes;

	#[test]
	fn cache_sizes_read_as_megabytes() {
		assert_eq!(megabytes(0), "0 MB");
		assert_eq!(megabytes(1536 * 1024), "1.5 MB");
		assert_eq!(megabytes(32 * 1024 * 1024), "32 MB");
	}
}
