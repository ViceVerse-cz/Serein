use super::*;

fn attachment(video: bool) -> Attachment {
	Attachment {
		id: Id(42),
		filename: if video { "clip.mp4" } else { "picture.png" }.into(),
		content_type: Some(if video { "video/mp4" } else { "image/png" }.into()),
		description: None,
		size: 512,
		spoiler: false,
		duration_ms: None,
		waveform: Vec::new(),
		media: model::EmbedMedia {
			width: 640,
			height: 360,
			..Default::default()
		},
	}
}

fn select_media_menu(
	attachment: &Attachment,
	in_viewer: bool,
	demo: bool,
	mut download: DownloadUi,
	label: &str,
	control: Option<(crate::video::VideoState, egui::Pos2)>,
	keyboard: bool,
) -> DownloadUi {
	let ctx = egui::Context::default();
	let mut message = test_support::message(1, Id(2));
	message.attachments.push(attachment.clone());
	let mut images = Avatars::default();
	let mut viewing = None;
	let mut opening = None;
	let mut video = crate::video::VideoUi::default();
	if let Some((state, _)) = control {
		video.active = Some((message.channel, message.id, attachment.clone()));
		video.state = state;
		video.duration = 12.0;
	}
	let mut audio = crate::audio::AudioUi::default();
	let mut frame = |events| {
		ctx.run_ui(
			egui::RawInput {
				screen_rect: Some(Rect::from_min_size(
					egui::Pos2::ZERO,
					egui::vec2(1000.0, 700.0),
				)),
				events,
				..Default::default()
			},
			|ui| {
				if in_viewer {
					assert_eq!(
						viewer(
							ui,
							&message.attachments,
							attachment.id,
							&mut images,
							&mut download,
							&mut opening,
							demo
						),
						Some(attachment.id)
					);
				} else {
					show(
						ui,
						&message,
						&mut images,
						&mut viewing,
						&mut opening,
						&mut download,
						&mut audio,
						&mut video,
						demo,
						&mut crate::select::Surface::new(ui, "attachment-test"),
					);
				}
			},
		)
	};
	frame(vec![]).drop_without_applying_deltas();
	frame(vec![]).drop_without_applying_deltas();
	let pos = if let Some((_, pos)) = control {
		pos
	} else if in_viewer {
		egui::pos2(500.0, 350.0)
	} else {
		egui::pos2(80.0, 80.0)
	};
	frame(vec![egui::Event::PointerMoved(pos)]).drop_without_applying_deltas();
	if keyboard {
		for (key, modifiers) in [
			(egui::Key::Tab, egui::Modifiers::NONE),
			(egui::Key::F10, egui::Modifiers::SHIFT),
		] {
			frame(vec![egui::Event::Key {
				key,
				physical_key: None,
				pressed: true,
				repeat: false,
				modifiers,
			}])
			.drop_without_applying_deltas();
		}
	} else {
		for pressed in [true, false] {
			frame(vec![
				egui::Event::PointerMoved(pos),
				egui::Event::PointerButton {
					pos,
					button: egui::PointerButton::Secondary,
					pressed,
					modifiers: egui::Modifiers::NONE,
				},
			])
			.drop_without_applying_deltas();
		}
	}
	let output = frame(vec![]);
	let target = output
		.shapes
		.iter()
		.find_map(|shape| match &shape.shape {
			egui::Shape::Text(text) if text.galley.job.text == label => {
				Some(text.pos + text.galley.rect.center().to_vec2())
			}
			_ => None,
		})
		.unwrap_or_else(|| panic!("right-click menu is missing {label}"));
	output.drop_without_applying_deltas();
	for pressed in [true, false] {
		frame(vec![
			egui::Event::PointerMoved(target),
			egui::Event::PointerButton {
				pos: target,
				button: egui::PointerButton::Primary,
				pressed,
				modifiers: egui::Modifiers::NONE,
			},
		])
		.drop_without_applying_deltas();
	}
	assert!(viewing.is_none() && opening.is_none());
	assert!(
		video.command.is_none(),
		"context click triggered {} at {control:?}",
		match &video.command {
			Some(crate::video::VideoCommand::Seek(_)) => "seek",
			Some(crate::video::VideoCommand::Volume(_)) => "volume",
			Some(crate::video::VideoCommand::Pause(_)) => "pause",
			Some(crate::video::VideoCommand::Play(_)) => "play",
			Some(crate::video::VideoCommand::Stop) => "stop",
			None => "no command",
		}
	);
	assert!(audio.command.is_none());
	assert!(images.take_requests().is_empty());
	download
}

#[test]
fn image_video_and_viewer_context_menus_copy_or_save_the_attachment() {
	for (video, in_viewer) in [(false, false), (true, false), (false, true)] {
		let attachment = attachment(video);
		let kind = if video { "video" } else { "image" };
		for copy in [true, false] {
			let label = if copy {
				format!("Copy {kind}")
			} else {
				format!("Save {kind} as…")
			};
			let download = select_media_menu(
				&attachment,
				in_viewer,
				false,
				DownloadUi::default(),
				&label,
				None,
				false,
			);
			assert_eq!(download.copy_request.as_ref(), copy.then_some(&attachment));
			assert_eq!(download.request.as_ref(), (!copy).then_some(&attachment));
		}
	}
}

#[test]
fn media_context_menus_disable_transfers_in_demo_and_while_busy() {
	for (video, in_viewer) in [(false, false), (true, false), (false, true)] {
		let attachment = attachment(video);
		let kind = if video { "video" } else { "image" };
		let mut previous = attachment.clone();
		previous.id = Id(99);
		for mode in 0..5 {
			for label in [format!("Copy {kind}"), format!("Save {kind} as…")] {
				let request = (mode == 2).then(|| previous.clone());
				let copy_request = (mode == 3).then(|| previous.clone());
				let embed_request = (mode == 4).then(|| (previous.media.clone(), true));
				let download = select_media_menu(
					&attachment,
					in_viewer,
					mode == 0,
					DownloadUi {
						active: mode == 1,
						request: request.clone(),
						copy_request: copy_request.clone(),
						embed_request: embed_request.clone(),
						..Default::default()
					},
					&label,
					None,
					false,
				);
				assert_eq!(download.request, request);
				assert_eq!(download.copy_request, copy_request);
				assert_eq!(download.embed_request, embed_request);
			}
		}
	}
}

#[test]
fn media_menus_open_from_keyboard() {
	for video in [false, true] {
		let attachment = attachment(video);
		let label = if video { "Copy video" } else { "Copy image" };
		let download = select_media_menu(
			&attachment,
			false,
			false,
			DownloadUi::default(),
			label,
			None,
			true,
		);
		assert_eq!(download.copy_request, Some(attachment));
	}
}

#[test]
fn media_menus_open_from_active_video_controls() {
	let attachment = attachment(true);
	for state in [
		crate::video::VideoState::Playing,
		crate::video::VideoState::Paused,
	] {
		for pos in [
			egui::pos2(200.0, 193.0),
			egui::pos2(21.0, 213.0),
			egui::pos2(375.0, 213.0),
		] {
			let download = select_media_menu(
				&attachment,
				false,
				false,
				DownloadUi::default(),
				"Copy video",
				Some((state, pos)),
				false,
			);
			assert_eq!(download.copy_request.as_ref(), Some(&attachment));
		}
	}
}

#[test]
fn video_controls_still_handle_primary_clicks() {
	use crate::video::{VideoCommand, VideoState, VideoUi};
	let attachment = attachment(true);
	let message = test_support::message(1, Id(2));
	for state in [VideoState::Playing, VideoState::Paused] {
		for (control, pos) in [
			egui::pos2(200.0, 193.0),
			egui::pos2(21.0, 213.0),
			egui::pos2(345.0, 213.0),
		]
		.into_iter()
		.enumerate()
		{
			let ctx = egui::Context::default();
			let mut video = VideoUi::default();
			video.active = Some((message.channel, message.id, attachment.clone()));
			video.state = state;
			video.duration = 12.0;
			let mut download = DownloadUi::default();
			let mut opening = None;
			let mut frame = |events| {
				ctx.run_ui(
					egui::RawInput {
						screen_rect: Some(Rect::from_min_size(
							egui::Pos2::ZERO,
							egui::vec2(1000.0, 700.0),
						)),
						events,
						..Default::default()
					},
					|ui| {
						let response = video.show(
							ui,
							&message,
							&attachment,
							&mut download,
							&mut opening,
							false,
						);
						media_context_menu(
							&response,
							&attachment,
							&mut download,
							&mut opening,
							false,
						);
					},
				)
				.drop_without_applying_deltas();
			};
			frame(vec![]);
			frame(vec![egui::Event::PointerMoved(pos)]);
			frame(vec![]);
			for pressed in [true, false] {
				frame(vec![egui::Event::PointerButton {
					pos,
					button: egui::PointerButton::Primary,
					pressed,
					modifiers: egui::Modifiers::NONE,
				}]);
			}
			match (control, video.command.take()) {
				(0, Some(VideoCommand::Seek(position))) => assert!(position > 0.0),
				(1, Some(VideoCommand::Pause(pause))) => {
					assert_eq!(pause, state == VideoState::Playing)
				}
				(2, Some(VideoCommand::Volume(volume))) => assert!(volume < 1.0),
				_ => panic!("primary click did not operate video control {control}"),
			}
			assert!(download.request.is_none() && download.copy_request.is_none());
			assert!(!egui::Popup::is_any_open(&ctx));
		}
	}
}
