//! Offline debug check: cargo run --locked -p ui --features demo --example gif_hover
use egui::{Color32, ColorImage, Pos2, RawInput, Rect, TextureId};
use std::{sync::Arc, time::Duration};

fn main() {
	let ctx = egui::Context::default();
	let mut state = test_support::demo_state();
	let mut view = ui::MessagingUi::default();
	view.apply_reading_preferences(&ctx, model::ReadingPreferences::default());
	view.preview_profile(test_support::message(1, model::Id(20)).author);
	let frame = |state: &mut client_core::State,
	             view: &mut ui::MessagingUi,
	             focused: bool,
	             pointer: Pos2| {
		ctx.run_ui(
			RawInput {
				focused,
				screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(1000.0, 760.0))),
				events: vec![egui::Event::PointerMoved(pointer)],
				..Default::default()
			},
			|ui| {
				view.show(ui, state);
			},
		)
	};
	frame(&mut state, &mut view, true, Pos2::ZERO).drop_without_applying_deltas();
	let profile = state.profile.as_mut().unwrap().data.as_mut().unwrap();
	profile.user.avatar = Some(format!("a_{}", "1".repeat(32)));
	profile.banner = Some(format!("a_{}", "2".repeat(32)));
	profile.guild = None;
	let keys = [profile.user.avatar_key(), profile.banner_key().unwrap()];
	// No worker is attached; service requests are only collected locally.
	state.demo = false;
	for _ in 0..3 {
		frame(&mut state, &mut view, true, Pos2::ZERO).drop_without_applying_deltas();
	}
	let requests = view.take_avatar_requests();
	for key in &keys {
		assert!(requests.contains(key));
		let still = ColorImage::filled([2, 2], Color32::RED);
		view.accept_avatar(&ctx, key.clone(), Some(still.clone()));
		view.accept_gif_animation(
			key.clone(),
			vec![
				(Duration::from_secs(1), Arc::new(still)),
				(
					Duration::from_secs(1),
					Arc::new(ColorImage::filled([2, 2], Color32::BLUE)),
				),
			],
		);
	}
	let output = frame(&mut state, &mut view, false, Pos2::ZERO);
	let still_ids: Vec<_> = output
		.textures_delta
		.set
		.iter()
		.filter(|(_, deltas)| deltas.iter().any(|delta| delta.image.size() == [2, 2]))
		.map(|(id, _)| *id)
		.collect();
	assert_eq!(still_ids.len(), 2, "unfocused animations must not upload");
	let artwork: Vec<_> = output
		.shapes
		.iter()
		.filter_map(|shape| match &shape.shape {
			egui::Shape::Rect(rect) if still_ids.contains(&rect.fill_texture_id()) => {
				Some(rect.rect)
			}
			_ => None,
		})
		.collect();
	assert_eq!(artwork.len(), 2, "avatar and banner are visible");
	output.drop_without_applying_deltas();
	let uses = |output: &egui::FullOutput, id: TextureId| {
		output
			.shapes
			.iter()
			.filter(
				|shape| matches!(&shape.shape, egui::Shape::Rect(rect) if rect.fill_texture_id() == id),
			)
			.count()
	};
	let output = frame(&mut state, &mut view, true, Pos2::ZERO);
	let animated_ids: Vec<_> = output
		.textures_delta
		.set
		.keys()
		.filter(|id| !still_ids.contains(id) && uses(&output, **id) > 0)
		.copied()
		.collect();
	assert_eq!(
		animated_ids.len(),
		2,
		"open profile animates without hovering"
	);
	output.drop_without_applying_deltas();
	for focused in [false, true] {
		let output = frame(&mut state, &mut view, focused, Pos2::ZERO);
		for id in &animated_ids {
			assert_eq!(uses(&output, *id), usize::from(focused));
			if !focused {
				assert!(!output.textures_delta.set.contains_key(id));
			}
		}
		output.drop_without_applying_deltas();
	}
	view.apply_reading_preferences(
		&ctx,
		model::ReadingPreferences {
			animate_gifs: false,
			..Default::default()
		},
	);
	for rect in artwork {
		let output = frame(&mut state, &mut view, true, rect.center());
		assert_eq!(
			still_ids.iter().map(|id| uses(&output, *id)).sum::<usize>(),
			2,
			"Animate GIFs off keeps both images still even while hovered"
		);
		output.drop_without_applying_deltas();
	}
	member_row();
	println!(
		"Open profiles autoplay; member rows animate on full-row hover; focus and Animate GIFs gate both."
	);
}

fn member_row() {
	let ctx = egui::Context::default();
	let mut state = test_support::demo_state();
	let channel = state.selected.unwrap();
	state.request_members();
	let mut message = test_support::message(500, channel);
	message.author.avatar = Some(format!("a_{}", "3".repeat(32)));
	message.content = "Shared avatar stays still in the timeline".into();
	message.attachments.clear();
	message.embeds.clear();
	let user = message.author.clone();
	let key = user.avatar_key();
	state.timeline.clear();
	state.timeline.insert(message, true, false).unwrap();
	let list = state.members.as_mut().unwrap();
	list.lazy = false;
	list.freshness = model::Freshness::Fresh;
	list.total = 1;
	list.slots = vec![Some(model::MemberSlot::Person(model::Member {
		user,
		nick: Some("Animated member".into()),
		roles: vec![],
		status: Some("online".into()),
		custom_status: None,
		activities: vec![],
	}))];
	state.demo = false;
	let mut view = ui::MessagingUi::default();
	view.apply_reading_preferences(
		&ctx,
		model::ReadingPreferences {
			show_members: true,
			..Default::default()
		},
	);
	let frame = |state: &mut client_core::State, view: &mut ui::MessagingUi, focused, pointer| {
		ctx.run_ui(
			RawInput {
				focused,
				screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(1200.0, 800.0))),
				events: vec![egui::Event::PointerMoved(pointer)],
				..Default::default()
			},
			|ui| {
				view.show(ui, state);
			},
		)
	};
	for _ in 0..3 {
		frame(&mut state, &mut view, true, Pos2::ZERO).drop_without_applying_deltas();
	}
	assert!(view.take_avatar_requests().contains(&key));
	let still = ColorImage::filled([2, 2], Color32::RED);
	view.accept_avatar(&ctx, key.clone(), Some(still.clone()));
	view.accept_gif_animation(
		key,
		vec![
			(Duration::from_secs(1), Arc::new(still)),
			(
				Duration::from_secs(1),
				Arc::new(ColorImage::filled([2, 2], Color32::BLUE)),
			),
		],
	);
	let output = frame(&mut state, &mut view, true, Pos2::ZERO);
	let still_id = *output
		.textures_delta
		.set
		.iter()
		.find(|(_, deltas)| deltas.iter().any(|delta| delta.image.size() == [2, 2]))
		.unwrap()
		.0;
	let artwork = |output: &egui::FullOutput, id: TextureId| -> Vec<Rect> {
		output
			.shapes
			.iter()
			.filter_map(|shape| match &shape.shape {
				egui::Shape::Rect(rect) if rect.fill_texture_id() == id => Some(rect.rect),
				_ => None,
			})
			.collect()
	};
	let rects = artwork(&output, still_id);
	assert!(rects.len() >= 2, "timeline and member list share an avatar");
	let member = rects
		.iter()
		.max_by(|a, b| a.left().total_cmp(&b.left()))
		.unwrap();
	let pointer = egui::pos2(member.right() + 80.0, member.center().y);
	assert!(!member.contains(pointer), "hover the name side of the row");
	output.drop_without_applying_deltas();
	let output = frame(&mut state, &mut view, true, pointer);
	let animated_id = *output
		.textures_delta
		.set
		.iter()
		.find(|(id, deltas)| {
			**id != still_id && deltas.iter().any(|delta| delta.image.size() == [2, 2])
		})
		.expect("hovering the row starts its avatar")
		.0;
	assert_eq!(artwork(&output, animated_id).len(), 1);
	assert!(
		!artwork(&output, still_id).is_empty(),
		"unhovered timeline copy stays still"
	);
	output.drop_without_applying_deltas();
	for (focused, pointer) in [(false, pointer), (true, Pos2::ZERO)] {
		let output = frame(&mut state, &mut view, focused, pointer);
		assert!(artwork(&output, animated_id).is_empty());
		assert!(!output.textures_delta.set.contains_key(&animated_id));
		output.drop_without_applying_deltas();
	}
	view.apply_reading_preferences(
		&ctx,
		model::ReadingPreferences {
			show_members: true,
			animate_gifs: false,
			..Default::default()
		},
	);
	let output = frame(&mut state, &mut view, true, pointer);
	assert!(artwork(&output, animated_id).is_empty());
	assert!(!output.textures_delta.set.contains_key(&animated_id));
	output.drop_without_applying_deltas();
}
