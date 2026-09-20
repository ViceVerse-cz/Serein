//! Offline debug check: cargo run --locked -p ui --example autoscroll
fn rows(shape: &egui::Shape, clip: egui::Rect, visible: &mut Vec<u64>) {
	match shape {
		egui::Shape::Text(text) if clip.contains(text.pos) => {
			if let Some(id) = text
				.galley
				.job
				.text
				.strip_prefix("Autoscroll row ")
				.and_then(|text| text.parse().ok())
			{
				visible.push(id);
			}
		}
		egui::Shape::Vec(shapes) => {
			for shape in shapes {
				rows(shape, clip, visible);
			}
		}
		_ => {}
	}
}

fn texts(shape: &egui::Shape, clip: egui::Rect, found: &mut Vec<(String, egui::Pos2)>) {
	match shape {
		egui::Shape::Text(text) if clip.intersects(text.visual_bounding_rect()) => {
			found.push((text.galley.job.text.clone(), text.pos));
		}
		egui::Shape::Vec(shapes) => {
			for shape in shapes {
				texts(shape, clip, found);
			}
		}
		_ => {}
	}
}

fn history_cursors(
	commands: &[client_core::Command],
) -> Option<(Option<model::Id>, Option<model::Id>)> {
	commands.iter().find_map(|command| match command {
		client_core::Command::History { before, after, .. } => Some((*before, *after)),
		_ => None,
	})
}

fn copied_text(output: &egui::FullOutput) -> String {
	output
		.platform_output
		.commands
		.iter()
		.find_map(|command| match command {
			egui::OutputCommand::CopyText(text) => Some(text.clone()),
			_ => None,
		})
		.unwrap_or_default()
}

fn clear_selection<F: FnMut(Vec<egui::Event>) -> u64>(frame: &mut F) {
	frame(vec![egui::Event::Key {
		key: egui::Key::Escape,
		physical_key: None,
		pressed: true,
		repeat: false,
		modifiers: egui::Modifiers::NONE,
	}]);
	frame(vec![]);
}

fn drag_select<F: FnMut(Vec<egui::Event>) -> u64>(
	frame: &mut F,
	copied: &std::cell::RefCell<String>,
	start: egui::Pos2,
	end: egui::Pos2,
) -> String {
	let press = |pos, pressed| egui::Event::PointerButton {
		pos,
		button: egui::PointerButton::Primary,
		pressed,
		modifiers: egui::Modifiers::NONE,
	};
	frame(vec![egui::Event::PointerMoved(start)]);
	frame(vec![press(start, true)]);
	frame(vec![egui::Event::PointerMoved(end)]);
	frame(vec![egui::Event::Copy]);
	let text = copied.borrow().clone();
	frame(vec![press(end, false)]);
	text
}

fn main() {
	assert_eq!(
		ui::scroll::speed(15.0),
		0.0,
		"inside the dead zone nothing moves"
	);
	let near = ui::scroll::speed(60.0);
	let far = ui::scroll::speed(240.0);
	assert!(
		far / near > 12.0,
		"autoscroll must accelerate superlinearly: {near} -> {far}"
	);

	let mut state = test_support::empty_channel_demo_state(false);
	let channel = state.selected.unwrap();
	for id in 1..=500 {
		let mut message = test_support::message(id, channel);
		message.content = format!("Autoscroll row {id}");
		message.embeds.clear();
		message.attachments.clear();
		message.reactions = Some(vec![]);
		state.timeline.insert(message, false, false).unwrap();
	}
	state
		.channels
		.iter_mut()
		.find(|entry| entry.id == channel)
		.unwrap()
		.last_message = Some(model::Id(500));
	let state = std::cell::RefCell::new(state);
	let ctx = egui::Context::default();
	ui::design::apply(&ctx);
	assert_eq!(
		ctx.options(|options| options.input_options.line_scroll_speed),
		120.0
	);
	let mut view = ui::MessagingUi::default();
	let mut number = 0;
	let middle = std::cell::Cell::new(ui::scroll::Middle::default());
	let repaint_delay = std::cell::Cell::new(std::time::Duration::ZERO);
	let any_down = std::cell::Cell::new(false);
	let has_selection = std::cell::Cell::new(false);
	let copied = std::cell::RefCell::new(String::new());
	let labels = std::cell::RefCell::new(Vec::<(String, egui::Pos2)>::new());
	let cursor = std::cell::Cell::new(egui::CursorIcon::Default);
	let commands = std::cell::RefCell::new(Vec::<client_core::Command>::new());
	let mut frame = |events| {
		number += 1;
		let mut feed = middle.get();
		view.middle_button(feed);
		feed.pressed = None;
		middle.set(feed);
		let output = ctx.run_ui(
			egui::RawInput {
				time: Some(f64::from(number) / 60.0),
				screen_rect: Some(egui::Rect::from_min_size(
					egui::Pos2::ZERO,
					egui::vec2(1120.0, 760.0),
				)),
				events,
				..Default::default()
			},
			|ui| {
				*commands.borrow_mut() = view.show(ui, &mut state.borrow_mut());
			},
		);
		repaint_delay.set(output.viewport_output[&egui::ViewportId::ROOT].repaint_delay);
		any_down.set(ctx.input(|input| input.pointer.any_down()));
		has_selection.set(
			ctx.plugin::<egui::text_selection::LabelSelectionState>()
				.lock()
				.has_selection(),
		);
		*copied.borrow_mut() = copied_text(&output);
		cursor.set(output.platform_output.cursor_icon);
		let mut visible = Vec::new();
		let mut found = Vec::new();
		for shape in &output.shapes {
			rows(&shape.shape, shape.clip_rect, &mut visible);
			texts(&shape.shape, shape.clip_rect, &mut found);
		}
		*labels.borrow_mut() = found;
		output.drop_without_applying_deltas();
		*visible.iter().max().expect("chat must remain visible")
	};
	for _ in 0..12 {
		frame(vec![]);
	}
	let origin = egui::pos2(600.0, 350.0);
	frame(vec![egui::Event::PointerMoved(origin)]);
	middle.set(ui::scroll::Middle {
		pressed: Some(origin),
		down: true,
	});
	frame(vec![]);
	frame(vec![egui::Event::PointerMoved(egui::pos2(600.0, 280.0))]);
	frame(vec![]);
	assert!(
		!any_down.get(),
		"the middle button must not reach egui's pointer state"
	);
	assert!(
		!has_selection.get(),
		"mouse3 must not start a label selection"
	);
	for (y, upward) in [(250.0, true), (450.0, false)] {
		let start = frame(vec![egui::Event::PointerMoved(egui::pos2(600.0, y))]);
		let mut previous = start;
		for _ in 0..60 {
			let current = frame(vec![]);
			assert!(
				current.abs_diff(previous) <= 5,
				"chat jumped: {previous} -> {current}"
			);
			assert!(
				if upward {
					current <= previous
				} else {
					current >= previous
				},
				"chat reversed: {previous} -> {current}"
			);
			previous = current;
		}
		assert!(
			if upward {
				previous < start
			} else {
				previous > start
			},
			"autoscroll must keep moving"
		);
	}
	frame(vec![egui::Event::PointerMoved(egui::pos2(600.0, -40.0))]);
	let edge = frame(vec![]);
	frame(vec![egui::Event::PointerGone]);
	let gone = frame(vec![]);
	assert!(
		gone < edge,
		"autoscroll must keep moving after the cursor leaves the window"
	);
	let injected = frame(vec![egui::Event::PointerMoved(egui::pos2(600.0, -2000.0))]);
	assert!(
		injected < gone,
		"an injected far pointer must keep increasing travel"
	);
	assert!(
		repaint_delay.get() < std::time::Duration::from_millis(16),
		"a live hold with a far pointer must keep requesting frames"
	);
	for y in [100.0, 600.0] {
		frame(vec![egui::Event::PointerMoved(egui::pos2(600.0, y))]);
		for _ in 0..300 {
			frame(vec![]);
		}
		assert!(
			repaint_delay.get() >= std::time::Duration::from_millis(16),
			"in-window at extent must not spin (pointer y={y})"
		);
	}
	frame(vec![egui::Event::PointerMoved(egui::pos2(600.0, 250.0))]);
	let before_release = frame(vec![]);
	let held = frame(vec![]);
	assert!(held < before_release, "autoscroll must move before release");
	middle.set(ui::scroll::Middle::default());
	frame(vec![]);
	let released = frame(vec![]);
	for _ in 0..30 {
		assert_eq!(
			frame(vec![]),
			released,
			"releasing mouse3 must stop autoscroll (held from {held})"
		);
	}
	frame(vec![egui::Event::PointerMoved(origin)]);
	middle.set(ui::scroll::Middle {
		pressed: Some(origin),
		down: true,
	});
	frame(vec![]);
	frame(vec![egui::Event::PointerMoved(egui::pos2(600.0, 250.0))]);
	let before_wheel = frame(vec![]);
	let moving = frame(vec![]);
	assert!(
		moving < before_wheel,
		"autoscroll must move before a wheel tick"
	);
	frame(vec![egui::Event::MouseWheel {
		unit: egui::MouseWheelUnit::Point,
		delta: egui::vec2(0.0, 40.0),
		modifiers: egui::Modifiers::NONE,
		phase: egui::TouchPhase::Move,
	}]);
	let after_wheel = frame(vec![]);
	for _ in 0..30 {
		assert_eq!(
			frame(vec![]),
			after_wheel,
			"a wheel tick must stop held autoscroll"
		);
	}
	middle.set(ui::scroll::Middle::default());
	frame(vec![]);
	frame(vec![egui::Event::PointerMoved(origin)]);
	middle.set(ui::scroll::Middle {
		pressed: Some(origin),
		down: true,
	});
	frame(vec![]);
	middle.set(ui::scroll::Middle::default());
	frame(vec![]);
	let latched = frame(vec![egui::Event::PointerMoved(egui::pos2(600.0, 250.0))]);
	let mut previous = latched;
	for _ in 0..60 {
		let current = frame(vec![]);
		assert!(
			current <= previous,
			"latched chat reversed: {previous} -> {current}"
		);
		previous = current;
	}
	assert!(
		previous < latched,
		"a mouse3 click must keep scrolling after release"
	);
	let off = egui::pos2(600.0, 250.0);
	frame(vec![egui::Event::PointerMoved(off)]);
	middle.set(ui::scroll::Middle {
		pressed: Some(off),
		down: true,
	});
	frame(vec![]);
	middle.set(ui::scroll::Middle::default());
	frame(vec![]);
	frame(vec![egui::Event::PointerMoved(egui::pos2(600.0, 150.0))]);
	let clicked_off = frame(vec![]);
	for _ in 0..30 {
		assert_eq!(
			frame(vec![]),
			clicked_off,
			"a second click must stop latched autoscroll, not start a new origin"
		);
	}
	let (row_id, row) = labels
		.borrow()
		.iter()
		.find_map(|(text, pos)| {
			text.strip_prefix("Autoscroll row ")
				.and_then(|rest| rest.parse::<u64>().ok())
				.map(|id| (id, *pos))
		})
		.expect("a chat row must stay visible for selection");
	let empty = drag_select(
		&mut frame,
		&copied,
		egui::pos2(860.0, 360.0),
		egui::pos2(860.0, 430.0),
	);
	assert!(
		empty.contains("Autoscroll row"),
		"empty space to the right of a short row must select chat text, got {empty:?}"
	);
	let from_avatar = drag_select(
		&mut frame,
		&copied,
		egui::pos2(row.x - 48.0, row.y - 8.0),
		egui::pos2(row.x + 240.0, row.y + 28.0),
	);
	assert!(
		from_avatar.contains("Autoscroll"),
		"a drag from the avatar column must select chat text, start={row:?} got {from_avatar:?}"
	);
	let from_header = drag_select(
		&mut frame,
		&copied,
		egui::pos2(row.x + 24.0, row.y - 18.0),
		egui::pos2(row.x + 240.0, row.y + 28.0),
	);
	assert!(
		from_header.contains("Autoscroll"),
		"a drag from the header band must select chat text, start={row:?} got {from_header:?}"
	);
	{
		let mut state = state.borrow_mut();
		if let Some(mut message) = state.timeline.get(model::Id(row_id)).cloned() {
			message.attachments = vec![model::Attachment {
				duration_ms: None,
				waveform: Vec::new(),
				id: model::Id(701),
				filename: "synthetic-notes.txt".into(),
				description: None,
				content_type: Some("text/plain".into()),
				size: 128,
				spoiler: false,
				media: model::EmbedMedia {
					url: Some(
						"https://cdn.discordapp.com/attachments/1/701/synthetic-notes.txt".into(),
					),
					..Default::default()
				},
			}];
			let _ = state.timeline.insert(message, false, false);
		}
		state.revision = state.revision.wrapping_add(1);
	}
	frame(vec![]);
	frame(vec![]);
	let file = labels
		.borrow()
		.iter()
		.find(|(text, _)| text.contains("synthetic-notes.txt"))
		.map(|(_, pos)| *pos)
		.unwrap_or(egui::pos2(row.x + 20.0, row.y + 40.0));
	let from_file = drag_select(
		&mut frame,
		&copied,
		file,
		egui::pos2(file.x + 220.0, file.y + 24.0),
	);
	assert!(
		from_file.contains("synthe") || from_file.contains("Autoscroll"),
		"a drag from a file row must select chat text, start={file:?} got {from_file:?}"
	);
	frame(vec![egui::Event::PointerMoved(row)]);
	assert_ne!(
		cursor.get(),
		egui::CursorIcon::Text,
		"chat selection must keep the default pointer, got {:?}",
		cursor.get()
	);
	{
		let mut state = state.borrow_mut();
		if let Some(mut message) = state.timeline.get(model::Id(row_id)).cloned() {
			message.reply_to = Some(model::Id(row_id.saturating_sub(1).max(1)));
			let _ = state.timeline.insert(message, false, false);
		}
		state.revision = state.revision.wrapping_add(1);
	}
	frame(vec![]);
	frame(vec![]);
	let reply = labels
		.borrow()
		.iter()
		.find(|(text, pos)| text.starts_with('@') && (pos.y - row.y).abs() < 80.0)
		.map(|(_, pos)| *pos)
		.unwrap_or(egui::pos2(row.x - 40.0, row.y - 36.0));
	let from_reply = drag_select(
		&mut frame,
		&copied,
		egui::pos2(reply.x - 48.0, reply.y),
		egui::pos2(row.x + 200.0, row.y + 16.0),
	);
	assert!(
		from_reply.contains("Autoscroll") || from_reply.contains('@'),
		"a drag from a reply row must select chat text, start={reply:?} row={row:?} got {from_reply:?}"
	);
	assert!(
		has_selection.get(),
		"a chat drag must leave a live label selection"
	);
	let hold = egui::pos2(row.x + 40.0, row.y + 8.0);
	let drag = egui::pos2(row.x + 180.0, row.y + 16.0);
	frame(vec![egui::Event::PointerMoved(hold)]);
	frame(vec![egui::Event::PointerButton {
		pos: hold,
		button: egui::PointerButton::Primary,
		pressed: true,
		modifiers: egui::Modifiers::NONE,
	}]);
	frame(vec![egui::Event::PointerMoved(drag)]);
	assert_ne!(
		cursor.get(),
		egui::CursorIcon::Text,
		"dragging chat text must keep the default pointer, got {:?}",
		cursor.get()
	);
	frame(vec![egui::Event::PointerButton {
		pos: drag,
		button: egui::PointerButton::Primary,
		pressed: false,
		modifiers: egui::Modifiers::NONE,
	}]);
	assert!(
		has_selection.get(),
		"releasing a chat drag must keep the selection"
	);
	frame(vec![egui::Event::PointerButton {
		pos: drag,
		button: egui::PointerButton::Secondary,
		pressed: true,
		modifiers: egui::Modifiers::NONE,
	}]);
	frame(vec![egui::Event::PointerButton {
		pos: drag,
		button: egui::PointerButton::Secondary,
		pressed: false,
		modifiers: egui::Modifiers::NONE,
	}]);
	assert!(
		has_selection.get(),
		"a right-click must keep the chat selection"
	);
	let copy = labels
		.borrow()
		.iter()
		.find(|(text, _)| text == "Copy")
		.map(|(_, pos)| *pos);
	let Some(copy) = copy else {
		panic!(
			"right-click must offer Copy, labels={:?}",
			labels
				.borrow()
				.iter()
				.map(|(text, _)| text.as_str())
				.collect::<Vec<_>>()
		);
	};
	frame(vec![egui::Event::PointerMoved(copy)]);
	frame(vec![egui::Event::PointerButton {
		pos: copy,
		button: egui::PointerButton::Primary,
		pressed: true,
		modifiers: egui::Modifiers::NONE,
	}]);
	frame(vec![egui::Event::PointerButton {
		pos: copy,
		button: egui::PointerButton::Primary,
		pressed: false,
		modifiers: egui::Modifiers::NONE,
	}]);
	assert!(
		copied.borrow().contains("Autoscroll") || copied.borrow().contains('@'),
		"Copy on the context menu must copy the selection, got {:?}",
		copied.borrow()
	);
	{
		let mut state = state.borrow_mut();
		let mut media = test_support::message(501, channel);
		media.id = model::Id((3 * 86_400_000) << 22);
		media.content.clear();
		media.embeds.clear();
		media.reactions = Some(vec![]);
		media.reply_to = Some(model::Id(row_id));
		media.attachments = vec![model::Attachment {
			duration_ms: None,
			waveform: Vec::new(),
			id: model::Id(802),
			filename: "synthetic-landscape.png".into(),
			description: Some("offline preview".into()),
			content_type: Some("image/png".into()),
			size: 2048,
			spoiler: false,
			media: model::EmbedMedia {
				url: Some(
					"https://cdn.discordapp.com/attachments/1/802/synthetic-landscape.png".into(),
				),
				width: 640,
				height: 240,
				..Default::default()
			},
		}];
		let _ = state.timeline.insert(media, false, false);
		state.revision = state.revision.wrapping_add(1);
	}
	for _ in 0..80 {
		frame(vec![egui::Event::MouseWheel {
			unit: egui::MouseWheelUnit::Point,
			delta: egui::vec2(0.0, -80.0),
			modifiers: egui::Modifiers::NONE,
			phase: egui::TouchPhase::Move,
		}]);
		if labels
			.borrow()
			.iter()
			.any(|(text, _)| text.contains("2015") || text.contains("synthetic-landscape"))
		{
			break;
		}
	}
	clear_selection(&mut frame);
	let mut previous_date: Option<f32> = None;
	for _ in 0..40 {
		frame(vec![]);
		let y = labels
			.borrow()
			.iter()
			.filter(|(text, _)| text.contains("January") && text.contains("2015"))
			.map(|(_, pos)| pos.y)
			.fold(None, |max: Option<f32>, y| {
				Some(max.map_or(y, |max| max.max(y)))
			});
		if let (Some(y), Some(prev)) = (y, previous_date)
			&& (y - prev).abs() < 0.5
		{
			break;
		}
		previous_date = y;
	}
	let date = labels
		.borrow()
		.iter()
		.filter(|(text, _)| text.contains("January") && text.contains("2015"))
		.max_by(|a, b| a.1.y.total_cmp(&b.1.y))
		.map(|(_, pos)| *pos);
	let Some(date) = date else {
		panic!(
			"date rule never became visible, labels={:?}",
			labels
				.borrow()
				.iter()
				.map(|(text, _)| text.as_str())
				.collect::<Vec<_>>()
		);
	};
	let from_date = drag_select(
		&mut frame,
		&copied,
		egui::pos2(date.x - 48.0, date.y + 8.0),
		egui::pos2(date.x + 64.0, date.y + 8.0),
	);
	assert!(
		from_date.contains("January")
			|| from_date.contains("2015")
			|| from_date.contains("Autoscroll")
			|| from_date.contains('@'),
		"a drag from a date rule must select chat text, start={date:?} got {from_date:?}"
	);
	clear_selection(&mut frame);
	let media = labels
		.borrow()
		.iter()
		.find(|(text, _)| text.contains("synthetic-landscape") || text.contains("offline preview"))
		.map(|(_, pos)| *pos)
		.unwrap_or(egui::pos2(date.x, date.y + 80.0));
	let from_media = drag_select(
		&mut frame,
		&copied,
		egui::pos2(media.x - 20.0, media.y + 30.0),
		egui::pos2(media.x + 180.0, media.y - 80.0),
	);
	assert!(
		from_media.contains("Autoscroll")
			|| from_media.contains("2015")
			|| from_media.contains('@'),
		"a drag from a media-only row must select chat text, start={media:?} got {from_media:?}"
	);

	{
		let mut state = state.borrow_mut();
		state.older_exhausted = false;
		state
			.channels
			.iter_mut()
			.find(|entry| entry.id == channel)
			.unwrap()
			.last_message = Some(model::Id(2000));
	}
	frame(vec![egui::Event::PointerMoved(origin)]);
	middle.set(ui::scroll::Middle {
		pressed: Some(origin),
		down: true,
	});
	frame(vec![]);
	frame(vec![egui::Event::PointerMoved(egui::pos2(600.0, -40.0))]);
	let mut older = None;
	for _ in 0..400 {
		frame(vec![]);
		if let Some((Some(before), None)) = history_cursors(&commands.borrow()) {
			older = Some(before);
			break;
		}
	}
	let older = older.expect("a hold above the window must keep requesting older history");
	test_support::load_page_with_cursors(&mut state.borrow_mut(), Some(older), None);
	let first = state.borrow().timeline.row_ids().next();
	frame(vec![egui::Event::PointerMoved(egui::pos2(600.0, 900.0))]);
	let mut newer = None;
	for _ in 0..400 {
		frame(vec![]);
		if let Some((None, Some(after))) = history_cursors(&commands.borrow()) {
			newer = Some(after);
			break;
		}
	}
	let newer = newer.expect("a hold must request newer history at the first forward page");
	test_support::load_page_with_cursors(&mut state.borrow_mut(), None, Some(newer));
	{
		let mut state = state.borrow_mut();
		for id in newer.0.saturating_add(1)..=newer.0.saturating_add(50) {
			if let Some(mut message) = state.timeline.get(model::Id(id)).cloned() {
				message.content = format!("Autoscroll row {id}");
				message.embeds.clear();
				message.attachments.clear();
				let _ = state.timeline.insert(message, false, false);
			}
		}
		state.revision = state.revision.wrapping_add(1);
	}
	let after_first = state.borrow().timeline.row_ids().next();
	assert!(
		after_first.is_some_and(|id| id < newer),
		"a newer page must keep earlier rows, first={after_first:?} after={newer}"
	);
	assert!(
		state.borrow().search_target.is_none() || state.borrow().search_target == first,
		"a newer page must not jump to the top of the chat"
	);
	let after_page = frame(vec![]);
	assert!(
		after_page > 100,
		"applying a newer page must not send the view to the top, got {after_page}"
	);
	let mut previous = after_page;
	for _ in 0..60 {
		let current = frame(vec![]);
		assert!(
			current >= previous || current.abs_diff(previous) <= 5,
			"drive died after a newer page: {previous} -> {current}"
		);
		previous = current;
	}
	assert!(
		previous >= after_page,
		"autoscroll must survive the newer page and keep moving down"
	);
	println!(
		"PASS: synthetic chat scrolls while mouse3 is held and stops on release; a wheel tick stops a hold; a click latches until the next click; no animation loop at either in-window boundary; mouse3 never selects; empty space, the avatar column, the header band, a file row, a reply row, a date rule, and a media row select chat text; the pointer stays default over chat; a right-click keeps the selection and Copy pastes it; a hold above the window still loads older history; a newer page appends and keeps the drive."
	);
}
