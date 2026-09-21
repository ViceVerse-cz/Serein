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
