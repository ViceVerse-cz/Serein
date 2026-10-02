//! Offline check: cargo run --locked -p serein --example scrolling_preferences
use eframe::egui;
use model::ReadingPreferences;

fn main() {
	let store = local_store::LocalStore::open(std::path::Path::new(":memory:")).unwrap();
	for smooth_scrolling in [false, true] {
		for scroll_speed_percent in [25, 100, 200, 300] {
			let preferences = ReadingPreferences {
				smooth_scrolling,
				scroll_speed_percent,
				..Default::default()
			};
			store.save_reading_preferences(preferences).unwrap();
			assert_eq!(store.reading_preferences().unwrap(), preferences);
			for unit in [egui::MouseWheelUnit::Point, egui::MouseWheelUnit::Line] {
				let ctx = egui::Context::default();
				ui::design::apply(&ctx);
				for frame in 0..2 {
					ctx.run_ui(
						egui::RawInput {
							time: Some(f64::from(frame) / 60.0),
							events: if frame == 0 {
								vec![egui::Event::MouseWheel {
									unit,
									delta: egui::vec2(0.0, -10.0),
									phase: egui::TouchPhase::Move,
									modifiers: egui::Modifiers::NONE,
								}]
							} else {
								vec![]
							},
							..Default::default()
						},
						|ui| {
							let before = ui.input(|input| input.smooth_scroll_delta().y);
							ui::scroll::apply_preferences(&ctx, preferences);
							let expected = if smooth_scrolling {
								before
							} else if frame != 0 {
								0.0
							} else if unit == egui::MouseWheelUnit::Line {
								-1200.0
							} else {
								-10.0
							} * (f32::from(scroll_speed_percent) / 100.0);
							assert_eq!(ui.input(|input| input.smooth_scroll_delta().y), expected);
						},
					)
					.drop_without_applying_deltas();
				}
			}
		}
	}
	for scroll_speed_percent in [0, 24, 301, u16::MAX] {
		assert!(
			store
				.save_reading_preferences(ReadingPreferences {
					scroll_speed_percent,
					..Default::default()
				})
				.is_err()
		);
	}
	store
		.save_reading_preferences(ReadingPreferences::default())
		.unwrap();
	assert_eq!(
		store.reading_preferences().unwrap(),
		ReadingPreferences::default()
	);
}
