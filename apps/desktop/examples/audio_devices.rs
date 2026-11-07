//! Debug device-picker discovery without Discord, audio streams, or microphone capture.
//! Run: cargo run --locked -p serein --example audio_devices
use eframe::egui;

fn frame(ctx: &egui::Context, messaging: &mut ui::MessagingUi, state: &mut client_core::State) {
	ctx.run_ui(
		egui::RawInput {
			screen_rect: Some(egui::Rect::from_min_size(
				egui::Pos2::ZERO,
				egui::vec2(1120.0, 760.0),
			)),
			..Default::default()
		},
		|ui| {
			let _ = messaging.show(ui, state);
		},
	)
	.drop_without_applying_deltas();
}

fn main() {
	let ctx = egui::Context::default();
	ui::fonts::install(&ctx);
	ui::design::apply(&ctx);
	let mut state = test_support::demo_state();
	let mut messaging = ui::MessagingUi::default();
	messaging.voice_available = true;
	messaging.preview_settings("voice");
	for _ in 0..3 {
		frame(&ctx, &mut messaging, &mut state);
	}
	assert!(
		!messaging.voice_refresh_devices,
		"offline demo must not request real devices"
	);
	// Synthetic state stays local: this example has no desktop/network adapter.
	state.demo = false;
	for _ in 0..3 {
		frame(&ctx, &mut messaging, &mut state);
	}
	assert!(
		std::mem::take(&mut messaging.voice_refresh_devices),
		"opening voice settings must request devices"
	);
	let devices = std::thread::spawn(discord_voice::audio::devices)
		.join()
		.expect("device worker completes")
		.expect("native enumeration succeeds");
	println!("Microphones: {}", devices.inputs.len());
	for (_, name) in &devices.inputs {
		println!("  {name}");
	}
	println!("Speakers: {}", devices.outputs.len());
	for (_, name) in &devices.outputs {
		println!("  {name}");
	}
	messaging.voice_inputs = devices.inputs;
	messaging.voice_outputs = devices.outputs;
	messaging.voice_device_status = ui::VOICE_DEVICES_LOADED;
	for _ in 0..3 {
		frame(&ctx, &mut messaging, &mut state);
	}
	assert!(
		!messaging.voice_refresh_devices,
		"idle frames must not repeatedly enumerate devices"
	);
	assert!(
		messaging.voice_input.is_none() && messaging.voice_output.is_none(),
		"discovery preserves the user's system-default selection"
	);
	println!("Device-picker debug check passed; no audio streams were opened.");
}
