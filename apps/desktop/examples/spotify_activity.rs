//! Offline debug check: cargo run --locked -p serein --features demo --example spotify_activity
use eframe::egui;
#[allow(dead_code)]
#[path = "../src/avatars.rs"]
mod avatars;

fn main() {
	let now = std::time::SystemTime::now()
		.duration_since(std::time::UNIX_EPOCH)
		.unwrap()
		.as_millis() as u64;
	let ctx = egui::Context::default();
	ui::design::apply(&ctx);
	let mut state = test_support::demo_state();
	let mut view = ui::MessagingUi::default();
	for (asset, end) in [
		(
			Some("spotify:ab67616d0000b2730123456789abcdef01234567"),
			Some(now + 110_000),
		),
		(None, Some(now + 110_000)),
		(Some("spotify:../../secret"), Some(now + 110_000)),
		(None, Some(now - 131_000)),
		(None, Some(now - 10_000)),
		(None, Some(model::MAX_ACTIVITY_TIMESTAMP + 1)),
		(None, None),
	] {
		let wire = serde_json::json!({"user":{"id":"1"},"activities":[{
			"type":2,"name":"Spotify","details":"Synthetic track","state":"Synthetic artist",
			"assets":{"large_image":asset},"timestamps":{"start":now - 130_000,"end":end}
		}]});
		let model::Patch::Value(activities) =
			discord_protocol::presence::decode(&serde_json::to_vec(&wire).unwrap())
				.unwrap()
				.activities
		else {
			panic!()
		};
		let activity = &activities[0];
		assert!(activity.valid());
		let artwork = asset.is_some_and(|id| id.starts_with("spotify:ab"));
		assert_eq!(activity.image.is_some(), artwork);
		assert_eq!(
			activity.ends_at.is_some(),
			end.is_some_and(|end| end > now - 130_000 && end <= model::MAX_ACTIVITY_TIMESTAMP)
		);
		state.set_local_game_activity(Some(activity.clone()));
		assert_eq!(state.local_game_activity(), Some(activity));
		let mut text = String::new();
		let mut large_image = false;
		for _ in 0..3 {
			let output = ctx.run_ui(
				egui::RawInput {
					screen_rect: Some(egui::Rect::from_min_size(
						egui::Pos2::ZERO,
						egui::vec2(1000.0, 760.0),
					)),
					..Default::default()
				},
				|ui| {
					view.show(ui, &mut state);
				},
			);
			text.clear();
			large_image = false;
			for shape in &output.shapes {
				match &shape.shape {
					egui::Shape::Text(shape) => {
						text.push_str(&shape.galley.job.text);
						text.push('\n');
					}
					egui::Shape::Mesh(mesh) => {
						let width = mesh.calc_bounds().width();
						// Artwork is 64px; the icon atlas expands its glyph cell to ~73px.
						large_image |= if artwork {
							(width - 64.0).abs() < 0.5
						} else {
							(width - 73.14).abs() < 0.5
						};
					}
					egui::Shape::Rect(rect) => {
						large_image |= artwork
							&& (rect.rect.width() - 64.0).abs() < 0.5
							&& rect.brush.is_some();
					}
					_ => {}
				}
			}
			output.drop_without_applying_deltas();
		}
		assert!(
			text.contains("Listening to Spotify")
				&& text.contains("Synthetic track")
				&& text.contains("Synthetic artist")
		);
		assert!(large_image, "large cover or Spotify fallback: {asset:?}");
		assert_eq!(text.contains("4:00"), end == Some(now + 110_000));
		if end == Some(now - 10_000) {
			assert_eq!(
				text.lines().filter(|line| *line == "2:00").count(),
				2,
				"completed track clamps to duration"
			);
		}
		assert!(
			view.take_avatar_requests().is_empty(),
			"no live artwork requests"
		);
	}
	println!(
		"Spotify artwork parsing, fallback, timing bounds and profile rendering passed (offline)."
	);
}
