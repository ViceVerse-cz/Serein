//! Offline debug check: cargo run --locked -p ui --features demo --example profile_layout
use client_core::{Envelope, Event, permissions};

fn main() {
	let mut state = test_support::demo_state();
	let mut view = ui::MessagingUi::default();
	view.preview_profile(test_support::message(1, model::Id(20)).author);
	let ctx = egui::Context::default();
	ui::design::apply(&ctx);
	let frame = |state: &mut client_core::State, view: &mut ui::MessagingUi| {
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
	};
	frame(&mut state, &mut view).drop_without_applying_deltas();
	for name in [
		"!    Name",
		"!   A very long synthetic display name that fills the entire profile header",
	] {
		let profile = state.profile.as_mut().unwrap().data.as_mut().unwrap();
		profile.global_name = Some(name.into());
		if let Some(guild) = &mut profile.guild {
			guild.nick = Some(name.into());
		}
		profile.clan.as_mut().unwrap().tag = "MEOW".into();
		for _ in 0..3 {
			let output = frame(&mut state, &mut view);
			let mut found = false;
			for shape in &output.shapes {
				if let egui::Shape::Text(text) = &shape.shape {
					assert!(!text.galley.job.text.contains("!   "));
					if text.galley.job.text.starts_with("! ") {
						let rect = ctx
							.memory(|m| m.area_rect(egui::Id::unique("user-profile-popout")))
							.unwrap();
						assert!(
							(text.pos.x - rect.left() - 24.0).abs() < 1.0,
							"profile name is left aligned"
						);
					}
					if text.galley.job.text == "MEOW" {
						assert_eq!(text.galley.rows.len(), 1);
						let rect = ctx
							.memory(|m| m.area_rect(egui::Id::unique("user-profile-popout")))
							.unwrap();
						assert!(text.pos.x + text.galley.size().x <= rect.right());
						found = true;
					}
				}
			}
			assert!(found, "clan tag is visible");
			output.drop_without_applying_deltas();
		}
	}
	let guild = state.permissions.guilds.values().next().unwrap().clone();
	for _ in 0..3 {
		state.apply(Envelope {
			generation: state.generation,
			event: Event::Permissions(permissions::Event::Guild(guild.clone())),
		});
		assert!(
			state.profile.as_ref().unwrap().data.is_some(),
			"unchanged permissions preserve the profile"
		);
	}
	let mut changed = guild;
	changed.owner = Some(model::Id(999));
	state.apply(Envelope {
		generation: state.generation,
		event: Event::Permissions(permissions::Event::Guild(changed)),
	});
	assert!(
		state.profile.is_none(),
		"changed permissions still invalidate the profile"
	);
	println!(
		"Profile whitespace, single-line clan tag and unchanged-permission refresh checks passed."
	);
}
