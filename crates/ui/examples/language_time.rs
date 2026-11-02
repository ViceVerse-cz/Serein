//! Offline debug check: cargo run --locked -p ui --features demo --example language_time
fn main() {
	use model::TimeFormat::{Auto, TwelveHour, TwentyFourHour};
	use ui::i18n::Language;

	let mut state = test_support::demo_state();
	let mut view = ui::MessagingUi::default();
	view.language = Language::English;
	view.preview_settings("language");
	let ctx = egui::Context::default();
	ui::design::apply(&ctx);
	let frame = |view: &mut ui::MessagingUi, state: &mut client_core::State| {
		ctx.run_ui(
			egui::RawInput {
				screen_rect: Some(egui::Rect::from_min_size(
					egui::Pos2::ZERO,
					egui::vec2(1000.0, 760.0),
				)),
				..Default::default()
			},
			|ui| {
				view.show(ui, state);
			},
		)
		.drop_without_applying_deltas();
	};
	let at = time::OffsetDateTime::UNIX_EPOCH.replace_time(time::Time::from_hms(14, 5, 9).unwrap());
	ui::local_time::set_format(TwentyFourHour, Language::English);
	for _ in 0..3 {
		frame(&mut view, &mut state);
	}
	assert_eq!(ui::local_time::clock(at), "2:05 PM");
	// A stable page must not reapply the formatter on every frame.
	ui::local_time::set_format(TwentyFourHour, Language::English);
	frame(&mut view, &mut state);
	assert_eq!(ui::local_time::clock(at), "14:05");
	for (language, format, expected) in [
		(Language::Czech, Auto, "14:05"),
		(Language::Czech, TwelveHour, "2:05 PM"),
		(Language::Czech, TwentyFourHour, "14:05"),
		(Language::English, Auto, "2:05 PM"),
	] {
		view.language = language;
		view.time_format = format;
		frame(&mut view, &mut state);
		assert_eq!(ui::local_time::clock(at), expected);
	}
	assert_eq!(ui::local_time::clock_seconds(at), "2:05:09 PM");
	println!("language/time initialization, stable frames and preference changes passed");
}
