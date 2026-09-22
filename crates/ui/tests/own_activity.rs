use ui::MessagingUi;

#[test]
fn own_profile_and_server_member_render_local_updates_and_clear_without_changing_peers() {
	for dark in [true, false] {
		for width in [760.0, 1120.0] {
			let ctx = egui::Context::default();
			ctx.set_theme(if dark {
				egui::ThemePreference::Dark
			} else {
				egui::ThemePreference::Light
			});
			ui::design::apply(&ctx);
			let mut state = test_support::demo_state();
			state.selected = Some(model::Id(20));
			let own = state.user.as_ref().unwrap().clone();
			let mut peer = own.clone();
			peer.id = model::Id(999);
			peer.name = "Peer (synthetic)".into();
			let game = |name: &str| model::RichActivity {
				kind: 0,
				name: name.into(),
				details: None,
				state: None,
				image: None,
				small_image: None,
				ends_at: None,
				started_at: None,
			};
			state.members = Some(model::MemberList {
				guild: Some(model::Id(10)),
				channel: model::Id(20),
				request: 1,
				total: 2,
				lazy: false,
				groups: vec![],
				ranges: vec![],
				freshness: model::Freshness::Fresh,
				start: 0,
				slots: [(own.clone(), "Other session game"), (peer, "Peer game")]
					.into_iter()
					.map(|(user, name)| {
						Some(model::MemberSlot::Person(model::Member {
							user,
							roles: vec![],
							nick: None,
							status: Some("idle".into()),
							custom_status: Some("Custom status".into()),
							activities: vec![game(name)],
						}))
					})
					.collect(),
			});
			let mut view = MessagingUi::default();
			view.reading_preferences.show_members = true;
			view.preview_profile(own);
			// Leave footer activity unset: two game labels must come from the profile and member row.
			for details in [Some("First beatmap"), Some("Second beatmap"), None] {
				state.set_local_game_activity(details.map(|details| model::RichActivity {
					details: Some(details.into()),
					state: Some("Solo".into()),
					..game("osu!")
				}));
				let mut painted = String::new();
				for _ in 0..3 {
					let output = ctx.run_ui(
						egui::RawInput {
							screen_rect: Some(egui::Rect::from_min_size(
								egui::Pos2::ZERO,
								egui::vec2(width, 900.0),
							)),
							..Default::default()
						},
						|ui| {
							view.show(ui, &mut state);
						},
					);
					painted.clear();
					fn collect(shape: &egui::Shape, text: &mut String) {
						match shape {
							egui::Shape::Text(shape) => {
								text.push_str(&shape.galley.job.text);
								text.push('\n');
							}
							egui::Shape::Vec(shapes) => {
								for shape in shapes {
									collect(shape, text);
								}
							}
							_ => {}
						}
					}
					for shape in &output.shapes {
						collect(&shape.shape, &mut painted);
					}
					assert!(output.platform_output.commands.is_empty());
					output.drop_without_applying_deltas();
				}
				assert!(painted.contains("Playing Peer game"), "{painted}");
				assert!(painted.contains("Custom status"), "{painted}");
				assert!(view.take_avatar_requests().is_empty());
				if let Some(details) = details {
					assert!(
						painted.contains("Playing osu!\n") && painted.contains("\nosu!\n"),
						"{painted}"
					);
					assert!(
						painted.contains(details) && painted.contains("Solo"),
						"{painted}"
					);
					assert!(!painted.contains("Playing Other session game"), "{painted}");
					if details == "Second beatmap" {
						assert!(!painted.contains("First beatmap"), "{painted}");
					}
				} else {
					assert!(
						!painted.contains("Playing osu!") && !painted.contains("Second beatmap"),
						"{painted}"
					);
					assert!(
						painted.contains("Playing Other session game\n")
							&& painted.contains("\nOther session game\n"),
						"{painted}"
					);
				}
			}
		}
	}
}

#[test]
fn own_activity_panel_and_setting_render_and_clear() {
	for dark in [true, false] {
		for width in [760.0, 1120.0] {
			let ctx = egui::Context::default();
			ctx.set_theme(if dark {
				egui::ThemePreference::Dark
			} else {
				egui::ThemePreference::Light
			});
			ui::design::apply(&ctx);
			let mut state = test_support::demo_state();
			let mut view = MessagingUi::default();
			view.own_game = Some("Playing osu!".into());
			for settings in [false, true] {
				if settings {
					view.preview_settings("activity");
				}
				for enabled in [true, false] {
					view.share_game_activity = enabled;
					// Let the immediate-mode panel finish its sizing pass.
					for frame in 0..3 {
						let output = ctx.run_ui(
							egui::RawInput {
								screen_rect: Some(egui::Rect::from_min_size(
									egui::Pos2::ZERO,
									egui::vec2(width, 760.0),
								)),
								..Default::default()
							},
							|ui| {
								view.show(ui, &mut state);
							},
						);
						let text = output
							.shapes
							.iter()
							.filter_map(|shape| match &shape.shape {
								egui::Shape::Text(text) => Some(text.galley.job.text.as_str()),
								_ => None,
							})
							.collect::<Vec<_>>()
							.join("\n");
						output.drop_without_applying_deltas();
						if frame == 2 {
							assert_eq!(text.contains("Playing osu!"), enabled, "{text}");
							if settings {
								assert!(text.contains("Share game activity"), "{text}");
							}
						}
					}
				}
			}
		}
	}
}
