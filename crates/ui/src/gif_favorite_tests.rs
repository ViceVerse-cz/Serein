use super::*;

#[test]
fn message_gif_stars_report_busy_sync_and_accept_explicit_retries() {
	fn contains_text(shape: &egui::Shape, label: &str) -> bool {
		match shape {
			egui::Shape::Text(text) => text.galley.text() == label,
			egui::Shape::Vec(shapes) => shapes.iter().any(|shape| contains_text(shape, label)),
			_ => false,
		}
	}
	for kind in ["attachment", "gallery", "embed"] {
		for writing in [false, true] {
			for favorited in [false, true] {
				let ctx = egui::Context::default();
				design::apply(&ctx);
				let mut state = test_support::demo_state();
				let channel = state.selected.unwrap();
				let mut message = test_support::message(500, channel);
				message.content = "Synthetic GIF favorite regression".into();
				message.embeds.clear();
				message.attachments.truncate(1);
				let attachment = &mut message.attachments[0];
				attachment.filename = "synthetic-wave.gif".into();
				attachment.content_type = Some("image/gif".into());
				attachment.media.url =
					Some("https://cdn.discordapp.com/attachments/1/700/synthetic-wave.gif".into());
				attachment.media.width = 320;
				attachment.media.height = 200;
				let media = attachment.media.clone();
				let page =
					(kind != "attachment").then_some("https://tenor.com/view/synthetic-wave");
				let gif = embeds::gif_for_media(&media, page, true).unwrap();
				if kind != "attachment" {
					message.attachments.clear();
					message.embeds = (0..if kind == "gallery" { 2 } else { 1 })
						.map(|_| model::Embed {
							kind: "image".into(),
							url: page.map(str::to_owned),
							image: Some(media.clone()),
							..Default::default()
						})
						.collect();
				}
				state.timeline.clear();
				state.timeline.insert(message, true, false).unwrap();
				let favorites = if favorited { vec![gif.clone()] } else { vec![] };
				state.restore_gif_favorites(favorites.clone());
				let Some(Command::GifFavorites { request, .. }) = state.request_gif_favorites()
				else {
					panic!("initial sync")
				};
				if writing {
					state.apply_gif_favorites(request, Ok(favorites));
					let other = model::Gif {
						url: "https://tenor.com/view/synthetic-other".into(),
						preview: "https://media.tenor.com/synthetic/other.gif".into(),
						..gif.clone()
					};
					assert!(state.toggle_gif_favorite(&other));
					assert!(state.take_gif_favorites_command().is_some());
				}
				let pending = state.gifs.sync_pending.unwrap();
				let mut view = MessagingUi::default();
				let frame = |state: &mut State, view: &mut MessagingUi, events| {
					ctx.run_ui(
						egui::RawInput {
							screen_rect: Some(egui::Rect::from_min_size(
								egui::Pos2::ZERO,
								egui::vec2(1120.0, 760.0),
							)),
							focused: true,
							events,
							..Default::default()
						},
						|root| {
							let _ = view.show(root, state);
						},
					)
				};
				let mut star = None;
				for _ in 0..5 {
					let output = frame(&mut state, &mut view, vec![]);
					star = output.shapes.iter().find_map(|s| match &s.shape {
						egui::Shape::Rect(r) if r.rect.size() == egui::Vec2::splat(30.0) => {
							Some(r.rect.center())
						}
						_ => None,
					});
					output.drop_without_applying_deltas();
				}
				let pos = star.expect("real message favorite star");
				let click = || {
					vec![
						egui::Event::PointerMoved(pos),
						egui::Event::PointerButton {
							pos,
							button: egui::PointerButton::Primary,
							pressed: true,
							modifiers: Default::default(),
						},
						egui::Event::PointerButton {
							pos,
							button: egui::PointerButton::Primary,
							pressed: false,
							modifiers: Default::default(),
						},
					]
				};
				frame(&mut state, &mut view, click()).drop_without_applying_deltas();
				let output = frame(&mut state, &mut view, vec![]);
				let label = i18n::translate("gif-favorites-sync-pending");
				assert!(
					output
						.shapes
						.iter()
						.any(|s| contains_text(&s.shape, &label)),
					"{kind}, writing={writing}, favorited={favorited}: rejected clicks must show retry feedback"
				);
				output.drop_without_applying_deltas();
				assert_eq!(state.is_gif_favorite(&gif), favorited);
				assert_eq!(state.gifs.sync_pending, Some(pending));
				assert!(state.take_gif_favorites_command().is_none());
				assert!(view.timeline.viewing.is_none());
				assert!(view.timeline.opening.is_none());
				state.apply_gif_favorites(pending, Ok(state.gifs.favorites.clone()));
				assert_eq!(
					state.is_gif_favorite(&gif),
					favorited,
					"no implicit replay after sync"
				);
				frame(&mut state, &mut view, click()).drop_without_applying_deltas();
				assert_eq!(
					state.is_gif_favorite(&gif),
					!favorited,
					"{kind}, writing={writing}, favorited={favorited}, star={pos:?}, pending={:?}",
					state.gifs.sync_pending
				);
				assert!(
					matches!(state.take_gif_favorites_command(), Some(Command::GifFavorites { change: Some((target, favorite)), .. }) if target.url == gif.url && favorite != favorited)
				);
				assert!(state.take_gif_favorites_command().is_none());
			}
		}
	}
}
