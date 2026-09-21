//! Offline component timings; no window, GPU, network or account access.
//! Build with `cargo test --release --locked -p ui --test startup_memory --no-run`,
//! then run each ignored test in the produced executable with `--nocapture`.
use std::time::Instant;

fn measure(install: impl Fn(&egui::Context)) {
	for run in 0..6 {
		let ctx = egui::Context::default();
		let start = Instant::now();
		install(&ctx);
		let elapsed = start.elapsed();
		if run > 0 {
			println!("sample {run}: {:.3} ms", elapsed.as_secs_f64() * 1000.0);
		}
	}
}

#[test]
#[ignore = "release component benchmark; one warmup and five measured installations"]
fn font_install() {
	measure(ui::fonts::install);
}

#[test]
#[ignore = "release component benchmark; one warmup and five measured installations"]
fn emoji_install() {
	measure(|ctx| ui::emoji::install(ctx).unwrap());
}

#[test]
#[ignore = "release component benchmark; one warmup and five measured frame batches"]
fn settled_font_frames() {
	let ctx = egui::Context::default();
	ui::fonts::install(&ctx);
	let mut galleys = Vec::new();
	ctx.run_ui(Default::default(), |ui| {
		for row in 0..200 {
			galleys.push(ui.painter().layout_no_wrap(
				format!(
					"Synthetic row {row}: {}",
					"Text with punctuation — café. ".repeat(16)
				),
				egui::FontId::proportional(14.0),
				egui::Color32::WHITE,
			));
		}
	})
	.drop_without_applying_deltas();
	for run in 0..6 {
		let start = Instant::now();
		for _ in 0..2_000 {
			ctx.run_ui(Default::default(), |ui| {
				for (row, galley) in galleys.iter().enumerate() {
					ui.painter().galley(
						egui::pos2(0.0, row as f32),
						galley.clone(),
						egui::Color32::WHITE,
					);
				}
			})
			.drop_without_applying_deltas();
		}
		if run > 0 {
			println!(
				"settled_font_frames sample {run}: {:.3} ms",
				start.elapsed().as_secs_f64() * 1000.0
			);
		}
	}
}
