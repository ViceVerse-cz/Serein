//! Offline speaking-ring check; no microphone, devices or network.
use client_core::voice::Phase;

fn main() {
	let mut state = test_support::voice_demo_state();
	let own = state.user.as_ref().unwrap().id;
	state
		.voice
		.roster
		.retain(|entry| entry.participant.user == own);
	state
		.voice
		.active
		.as_mut()
		.unwrap()
		.participants
		.retain(|p| p.user == own);
	let mut view = ui::MessagingUi::default();
	view.voice_speaking.push(own);
	for (phase, muted, deafened, expected) in [
		(Phase::Waiting, false, false, true),
		(Phase::Connected, false, false, true),
		(Phase::Waiting, true, false, false),
		(Phase::Waiting, false, true, false),
		(Phase::Securing, false, false, false),
	] {
		let call = state.voice.active.as_mut().unwrap();
		call.phase = phase;
		call.deafened = deafened;
		call.participants[0].muted = muted;
		state.voice.roster[0].participant.muted = muted;
		let ctx = egui::Context::default();
		ctx.enable_accesskit();
		ui::design::apply(&ctx);
		let mut speaking = false;
		for _ in 0..3 {
			let output = ctx.run_ui(
				egui::RawInput {
					screen_rect: Some(egui::Rect::from_min_size(
						egui::Pos2::ZERO,
						egui::vec2(1120.0, 760.0),
					)),
					..Default::default()
				},
				|ui| {
					let _ = view.show(ui, &mut state);
				},
			);
			speaking = output
				.platform_output
				.accesskit_update
				.as_ref()
				.unwrap()
				.nodes
				.iter()
				.any(|(_, node)| {
					node.label()
						.is_some_and(|label| label.ends_with(" · Speaking"))
				});
			output.drop_without_applying_deltas();
		}
		assert_eq!(
			speaking, expected,
			"phase={phase:?}, muted={muted}, deafened={deafened}"
		);
	}
	println!(
		"Solo speaking indicator passed: waiting/connected glow, mute/deafen/security gates (offline)."
	);
}
