//! Offline debug check: cargo run --locked -p ui --features demo --example image_resolution
use egui::{Color32, ColorImage};
use model::Id;
use std::{sync::Arc, time::Duration};

fn frame(ctx: &egui::Context, view: &mut ui::MessagingUi, state: &mut client_core::State) {
	ctx.run_ui(
		egui::RawInput {
			screen_rect: Some(egui::Rect::from_min_size(
				egui::Pos2::ZERO,
				egui::vec2(1000.0, 800.0),
			)),
			..Default::default()
		},
		|ui| {
			let _ = view.show(ui, state);
		},
	)
	.drop_without_applying_deltas();
}

fn main() {
	for animated in [false, true] {
		let ctx = egui::Context::default();
		let mut state = test_support::demo_state();
		// No worker is attached: requests are inspected, never sent.
		state.demo = false;
		let mut message = test_support::message(500, Id(20));
		message.embeds.clear();
		message.attachments.truncate(1);
		let attachment = &mut message.attachments[0];
		attachment.filename = "still.WeBp".into();
		attachment.content_type = Some("image/webp".into());
		attachment.media.url =
			Some("https://cdn.discordapp.com/attachments/1/700/still.WeBp".into());
		attachment.media.width = 4096;
		attachment.media.height = 2048;
		state.timeline.insert(message, true, false).unwrap();
		let mut view = ui::MessagingUi::default();
		view.apply_reading_preferences(
			&ctx,
			model::ReadingPreferences {
				animate_gifs: true,
				..Default::default()
			},
		);
		view.preview_image_viewer(Id(500), Id(700));
		for _ in 0..3 {
			frame(&ctx, &mut view, &mut state);
		}
		let key = view
			.take_avatar_requests()
			.into_iter()
			.find(|key| key.starts_with("anim:") && key.contains("still.WeBp"))
			.expect("viewer requests the animation candidate");
		let image = Arc::new(ColorImage::filled([160, 80], Color32::WHITE));
		view.accept_avatar(&ctx, key.clone(), Some(image.as_ref().clone()));
		view.accept_gif_animation(
			key,
			vec![(Duration::from_millis(100), image); if animated { 2 } else { 1 }],
		);
		frame(&ctx, &mut view, &mut state);
		let large = view
			.take_avatar_requests()
			.into_iter()
			.find(|key| key.starts_with("large:") && key.contains("still.WeBp"));
		assert_eq!(large.is_some(), !animated, "only still images upgrade");
		if let Some(key) = large {
			assert!(key.ends_with("width=2048&height=1024"));
			view.accept_avatar(
				&ctx,
				key,
				Some(ColorImage::filled([2048, 1024], Color32::WHITE)),
			);
			frame(&ctx, &mut view, &mut state);
			assert!(view.take_avatar_requests().is_empty());
		}
	}
	println!("Still WebP upgrades to 2048px; animated WebP keeps playback.");
}
