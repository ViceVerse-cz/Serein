use super::*;

fn state() -> State {
	let mut owner = test_support::message(1, Id(10)).author;
	owner.id = Id(1);
	State {
		user: Some(owner),
		auth: client_core::auth::AuthState::Authenticated,
		gateway_connected: true,
		freshness: Freshness::Fresh,
		selected: Some(Id(10)),
		channels: vec![model::Channel {
			id: Id(10),
			guild: None,
			parent_id: None,
			position: 0,
			name: "Synthetic search conversation".into(),
			kind: 1,
			recipients: vec![test_support::message(2, Id(10)).author],
			icon: None,
			member_list_id: None,
			tags: None,
			message_count: None,
			last_message: None,
		}],
		..State::default()
	}
}
fn key(key: egui::Key, modifiers: egui::Modifiers) -> egui::Event {
	egui::Event::Key {
		key,
		physical_key: None,
		pressed: true,
		repeat: false,
		modifiers,
	}
}
fn primary() -> egui::Modifiers {
	egui::Modifiers {
		command: true,
		ctrl: !cfg!(target_os = "macos"),
		mac_cmd: cfg!(target_os = "macos"),
		..Default::default()
	}
}
fn frame(
	ctx: &egui::Context,
	view: &mut MessagingUi,
	state: &mut State,
	events: Vec<egui::Event>,
	pane: bool,
	modal: bool,
) -> (Vec<Command>, Vec<(String, egui::Rect)>) {
	frame_with_width(ctx, view, state, events, pane, modal, 420.0)
}
fn frame_with_width(
	ctx: &egui::Context,
	view: &mut MessagingUi,
	state: &mut State,
	events: Vec<egui::Event>,
	pane: bool,
	modal: bool,
	pane_width: f32,
) -> (Vec<Command>, Vec<(String, egui::Rect)>) {
	let mut commands = Vec::new();
	let output = ctx.run_ui(
		egui::RawInput {
			screen_rect: Some(egui::Rect::from_min_size(
				egui::Pos2::ZERO,
				if pane {
					egui::vec2(pane_width, 700.0)
				} else {
					egui::vec2(1200.0, 800.0)
				},
			)),
			events,
			..Default::default()
		},
		|ui| {
			if modal {
				egui::Modal::new(egui::Id::unique("synthetic-search-guard")).show(ctx, |ui| {
					ui.label("Synthetic confirmation");
				});
			}
			if pane {
				view.search.sync(ctx, state, &mut commands);
				view.search.pane(
					ui,
					state,
					&mut commands,
					&mut view.avatars,
					search::MediaUi {
						download: &mut view.timeline.download,
						audio: &mut view.timeline.audio,
						video: &mut view.timeline.video,
					},
					&mut view.profile,
				);
			} else {
				commands.extend(view.show(ui, state));
			}
		},
	);
	fn collect(shape: &egui::Shape, labels: &mut Vec<(String, egui::Rect)>) {
		match shape {
			egui::Shape::Text(text) => labels.push((
				text.galley.text().to_owned(),
				egui::Rect::from_min_size(text.pos, text.galley.size()),
			)),
			egui::Shape::Vec(shapes) => {
				for shape in shapes {
					collect(shape, labels);
				}
			}
			_ => {}
		}
	}
	let mut labels = Vec::new();
	for shape in &output.shapes {
		collect(&shape.shape, &mut labels);
	}
	output.drop_without_applying_deltas();
	ctx.input_mut(|input| input.keys_down.clear());
	(commands, labels)
}
fn page(state: &mut State, total: u64) {
	page_with(state, total, "Synthetic result to revisit");
}
fn page_with(state: &mut State, total: u64, excerpt: &str) {
	let request = state.search.as_ref().unwrap().request;
	state.apply_search(
		Id(10),
		request,
		Ok(client_core::search::Outcome::Page(model::SearchPage {
			hits: vec![model::SearchHit {
				id: Id(90),
				channel: Id(10),
				author: test_support::message(90, Id(10)).author,
				mentions: vec![],
				excerpt: excerpt.into(),
				attachments: vec![],
				embeds: vec![],
			}],
			total,
			partial: false,
			pin_cursor: None,
		})),
	);
}
fn click(
	ctx: &egui::Context,
	view: &mut MessagingUi,
	state: &mut State,
	point: egui::Pos2,
) -> Vec<Command> {
	let mut commands = Vec::new();
	for pressed in [true, false] {
		commands.extend(
			frame(
				ctx,
				view,
				state,
				vec![
					egui::Event::PointerMoved(point),
					egui::Event::PointerButton {
						pos: point,
						button: egui::PointerButton::Primary,
						pressed,
						modifiers: egui::Modifiers::NONE,
					},
				],
				true,
				false,
			)
			.0,
		);
	}
	commands
}

#[test]
fn conversation_search_shortcut_focuses_from_composer_and_preserves_results() {
	let ctx = egui::Context::default();
	let mut state = state();
	let mut view = MessagingUi::default();
	for _ in 0..3 {
		frame(&ctx, &mut view, &mut state, vec![], false, false);
	}
	frame(
		&ctx,
		&mut view,
		&mut state,
		vec![egui::Event::Text("Draft stays".into())],
		false,
		false,
	);
	assert_eq!(
		state.drafts.get(&Id(10)).map(String::as_str),
		Some("Draft stays")
	);
	frame(
		&ctx,
		&mut view,
		&mut state,
		vec![key(egui::Key::F, primary())],
		false,
		false,
	);
	assert!(view.search.open);
	frame(
		&ctx,
		&mut view,
		&mut state,
		vec![egui::Event::Text("needle".into())],
		false,
		false,
	);
	let (commands, _) = frame(
		&ctx,
		&mut view,
		&mut state,
		vec![key(egui::Key::Enter, egui::Modifiers::NONE)],
		false,
		false,
	);
	assert!(commands.iter().any(|command| matches!(command, Command::Search { channel: Id(10), guild: None, query, offset: 0, .. } if query == "needle")));
	assert!(
		!commands
			.iter()
			.any(|command| matches!(command, Command::Send { .. }))
	);
	page(&mut state, 75);
	state.request_search_page(1).unwrap();
	page(&mut state, 75);
	let request = state.search.as_ref().unwrap().request;
	frame(
		&ctx,
		&mut view,
		&mut state,
		vec![key(egui::Key::F, primary())],
		false,
		false,
	);
	let search = state.search.as_ref().unwrap();
	assert_eq!(
		(search.request, search.offset, search.query.as_str()),
		(request, 25, "needle")
	);
	assert_eq!(search.page.as_ref().unwrap().hits[0].id, Id(90));
	assert_eq!(state.drafts[&Id(10)], "Draft stays");
}

#[test]
fn conversation_search_shortcut_respects_guards_and_remapping() {
	for guard in 0..4 {
		let ctx = egui::Context::default();
		let mut state = state();
		let mut view = MessagingUi::default();
		frame(&ctx, &mut view, &mut state, vec![], false, false);
		view.settings.open = guard == 0;
		view.ime_active = guard == 1;
		if guard == 3 {
			frame(&ctx, &mut view, &mut state, vec![], false, true);
		}
		let mut events = vec![key(egui::Key::F, primary())];
		if guard == 2 {
			events.insert(
				0,
				egui::Event::Ime(egui::ImeEvent::Preedit {
					text: "語".into(),
					active_range_chars: None,
				}),
			);
		}
		frame(&ctx, &mut view, &mut state, events, false, guard == 3);
		assert!(!view.search.open, "guard {guard} opened background search");
		assert!(state.search.is_none());
	}
	let ctx = egui::Context::default();
	let mut state = state();
	let mut view = MessagingUi::default();
	view.keybinds.search_conversation = model::KeyChord::new("J", model::keybinds::PRIMARY);
	frame(&ctx, &mut view, &mut state, vec![], false, false);
	frame(
		&ctx,
		&mut view,
		&mut state,
		vec![key(egui::Key::F, primary())],
		false,
		false,
	);
	assert!(!view.search.open);
	frame(
		&ctx,
		&mut view,
		&mut state,
		vec![key(egui::Key::J, primary())],
		false,
		false,
	);
	assert!(view.search.open);
	let ctx = egui::Context::default();
	let mut state = self::state();
	let mut view = MessagingUi::default();
	frame(&ctx, &mut view, &mut state, vec![], false, false);
	frame(
		&ctx,
		&mut view,
		&mut state,
		vec![
			egui::Event::Ime(egui::ImeEvent::Preedit {
				text: String::new(),
				active_range_chars: None,
			}),
			key(egui::Key::F, primary()),
		],
		false,
		false,
	);
	assert!(
		view.search.open,
		"idle IME must not block the search shortcut"
	);
}

#[test]
fn result_jump_and_numeric_pager_keep_scope_and_allow_explicit_retry() {
	let ctx = egui::Context::default();
	let mut state = state();
	let mut view = MessagingUi::default();
	view.search
		.open_extension(Id(10), false, Some("needle".into()));
	state.request_search("needle".into(), None).unwrap();
	page(&mut state, 75);
	let request = state.search.as_ref().unwrap().request;
	let mut labels = Vec::new();
	for _ in 0..3 {
		labels = frame(&ctx, &mut view, &mut state, vec![], true, false).1;
	}
	let result = labels
		.iter()
		.find(|(text, _)| text == "Synthetic result to revisit")
		.unwrap()
		.1;
	// Hovering the result exposes its explicit jump control above the selectable text.
	let labels = frame(
		&ctx,
		&mut view,
		&mut state,
		vec![egui::Event::PointerMoved(result.center())],
		true,
		false,
	)
	.1;
	let jump = labels
		.iter()
		.find(|(text, _)| text == "Jump")
		.unwrap()
		.1
		.center();
	let commands = click(&ctx, &mut view, &mut state, jump);
	assert!(commands.iter().any(|command| matches!(
		command,
		Command::History {
			channel: Id(10),
			before: Some(Id(91)),
			..
		}
	)));
	assert!(view.search.open);
	assert_eq!(state.search.as_ref().unwrap().request, request);
	assert_eq!(state.search.as_ref().unwrap().query, "needle");
	let page_button = |labels: &[(String, egui::Rect)], page: &str| {
		labels
			.iter()
			.find(|(text, _)| text == page)
			.unwrap_or_else(|| panic!("page {page} in {labels:?}"))
			.1
			.center()
	};
	let labels = frame(&ctx, &mut view, &mut state, vec![], true, false).1;
	let loaded = page_button(&labels, "1");
	let commands = click(&ctx, &mut view, &mut state, page_button(&labels, "3"));
	assert!(commands.iter().any(|command| matches!(command, Command::Search { channel: Id(10), query, before: None, offset: 50, .. } if query == "needle")));
	// The next page has no results yet; the pager must stay at the pane's foot.
	let labels = frame(&ctx, &mut view, &mut state, vec![], true, false).1;
	let loading = page_button(&labels, "1");
	assert!(
		(loading.y - loaded.y).abs() < 1.0 && loaded.y > 600.0,
		"pager moved from {loaded:?} to {loading:?} while the page loaded"
	);
	let loading_request = state.search.as_ref().unwrap().request;
	let commands = frame(
		&ctx,
		&mut view,
		&mut state,
		vec![key(egui::Key::Enter, egui::Modifiers::NONE)],
		true,
		false,
	)
	.0;
	assert!(
		!commands
			.iter()
			.any(|command| matches!(command, Command::Search { .. }))
	);
	state.apply_search(
		Id(10),
		loading_request,
		Err(client_core::auth::Failure::Network),
	);
	let labels = frame(&ctx, &mut view, &mut state, vec![], true, false).1;
	let commands = click(&ctx, &mut view, &mut state, page_button(&labels, "2"));
	assert!(
		commands
			.iter()
			.any(|command| matches!(command, Command::Search { offset: 25, .. }))
	);
	page(&mut state, 75);
	for dark in [false, true] {
		ctx.set_visuals(if dark {
			egui::Visuals::dark()
		} else {
			egui::Visuals::light()
		});
		for width in [200.0, 220.0] {
			let mut labels = Vec::new();
			for _ in 0..3 {
				labels =
					frame_with_width(&ctx, &mut view, &mut state, vec![], true, false, width).1;
			}
			let pager: Vec<_> = labels
				.iter()
				.filter(|(text, _)| matches!(text.as_str(), "1" | "2" | "3"))
				.collect();
			assert_eq!(pager.len(), 3, "every page stays visible: {labels:?}");
			assert!(
				pager.iter().all(|(_, rect)| rect.left() >= 0.0
					&& rect.right() <= width
					&& rect.bottom() <= 700.0),
				"pager exceeds {width}px pane: {pager:?}"
			);
		}
	}
}

/// One results-pane frame that returns the drawn text and the text it copied.
fn pane_frame(
	ctx: &egui::Context,
	view: &mut MessagingUi,
	state: &mut State,
	events: Vec<egui::Event>,
) -> (Vec<(String, egui::Rect)>, Vec<String>) {
	let mut commands = Vec::new();
	let output = ctx.run_ui(
		egui::RawInput {
			screen_rect: Some(egui::Rect::from_min_size(
				egui::Pos2::ZERO,
				egui::vec2(420.0, 700.0),
			)),
			events,
			..Default::default()
		},
		|ui| {
			view.search.sync(ctx, state, &mut commands);
			view.search.pane(
				ui,
				state,
				&mut commands,
				&mut view.avatars,
				search::MediaUi {
					download: &mut view.timeline.download,
					audio: &mut view.timeline.audio,
					video: &mut view.timeline.video,
				},
				&mut view.profile,
			);
		},
	);
	let copied = output
		.platform_output
		.commands
		.iter()
		.filter_map(|command| match command {
			egui::OutputCommand::CopyText(text) => Some(text.clone()),
			_ => None,
		})
		.collect();
	let mut labels = Vec::new();
	let mut stack: Vec<_> = output.shapes.iter().map(|clipped| &clipped.shape).collect();
	while let Some(shape) = stack.pop() {
		match shape {
			egui::Shape::Text(text) => labels.push((
				text.galley.text().to_owned(),
				egui::Rect::from_min_size(text.pos, text.galley.size()),
			)),
			egui::Shape::Vec(shapes) => stack.extend(shapes),
			_ => {}
		}
	}
	output.drop_without_applying_deltas();
	(labels, copied)
}

/// Checks that right-clicking a result opens its menu, that each copy entry copies the
/// result's text, ID or link, and that a plugin message action is requested for that message.
#[test]
fn result_context_menu_copies_and_offers_plugin_message_actions() {
	let ctx = egui::Context::default();
	let mut state = state();
	let mut view = MessagingUi::default();
	view.search
		.open_extension(Id(10), false, Some("needle".into()));
	state.request_search("needle".into(), None).unwrap();
	page(&mut state, 1);
	let plugin = crate::extensions_ui::MenuAction {
		plugin: "synthetic.words".into(),
		action: "count".into(),
		label: "Synthetic · Count words".into(),
	};
	view.search.extension_actions = std::sync::Arc::new(vec![plugin.clone()]);
	let mut labels = Vec::new();
	for _ in 0..3 {
		labels = pane_frame(&ctx, &mut view, &mut state, vec![]).0;
	}
	let result = labels
		.iter()
		.find(|(text, _)| text == "Synthetic result to revisit")
		.expect("result is drawn")
		.1;
	// The card's own background, right of the excerpt, receives the secondary click.
	let card = egui::pos2(result.right() + 8.0, result.center().y);
	let press = |button, at: egui::Pos2| {
		[true, false]
			.into_iter()
			.map(|pressed| egui::Event::PointerButton {
				pos: at,
				button,
				pressed,
				modifiers: egui::Modifiers::NONE,
			})
			.collect::<Vec<_>>()
	};
	let open = |view: &mut MessagingUi, state: &mut State| {
		// A menu closed by the last click still counts as open for one frame.
		pane_frame(&ctx, view, state, vec![]);
		let mut events = vec![egui::Event::PointerMoved(card)];
		events.extend(press(egui::PointerButton::Secondary, card));
		pane_frame(&ctx, view, state, events);
		pane_frame(&ctx, view, state, vec![]).0
	};
	let entry = |labels: &[(String, egui::Rect)], key: &str| {
		let label = crate::i18n::translate(key);
		labels
			.iter()
			.find(|(text, _)| *text == label)
			.unwrap_or_else(|| panic!("{label} in menu"))
			.1
			.center()
	};
	for (key, expected) in [
		(
			"message-menu-copy",
			"Synthetic result to revisit".to_owned(),
		),
		("timeline-message-actions-copy-message-id", "90".to_owned()),
		(
			"timeline-message-actions-copy-message-link",
			"https://discord.com/channels/@me/10/90".to_owned(),
		),
	] {
		let labels = open(&mut view, &mut state);
		let (_, copied) = pane_frame(
			&ctx,
			&mut view,
			&mut state,
			press(egui::PointerButton::Primary, entry(&labels, key)),
		);
		assert_eq!(copied, vec![expected], "{key}");
	}
	assert!(view.search.extension_request.is_none());
	let labels = open(&mut view, &mut state);
	let submenu = entry(&labels, "timeline-message-actions-extensions");
	let mut events = vec![egui::Event::PointerMoved(submenu)];
	events.extend(press(egui::PointerButton::Primary, submenu));
	let mut labels = pane_frame(&ctx, &mut view, &mut state, events).0;
	for _ in 0..3 {
		if labels.iter().any(|(text, _)| *text == plugin.label) {
			break;
		}
		labels = pane_frame(&ctx, &mut view, &mut state, vec![]).0;
	}
	let item = labels
		.iter()
		.find(|(text, _)| *text == plugin.label)
		.expect("plugin action listed")
		.1
		.center();
	pane_frame(
		&ctx,
		&mut view,
		&mut state,
		press(egui::PointerButton::Primary, item),
	);
	let (action, text, target) = view
		.search
		.extension_request
		.clone()
		.expect("plugin action requested");
	// A full client frame hands the request to the plugin host with the message's IDs.
	frame(&ctx, &mut view, &mut state, vec![], false, false);
	assert!(view.search.extension_request.is_none());
	let [crate::extensions_ui::ExtensionRequest::Invoke { id, invocation, .. }] =
		view.extensions.requests.as_slice()
	else {
		panic!("one plugin invocation queued")
	};
	assert_eq!(id, "synthetic.words");
	assert_eq!(invocation.action, "count");
	assert_eq!(
		invocation.selected_message.as_deref(),
		Some("Synthetic result to revisit")
	);
	let reference = invocation.selected_message_ref.as_deref().unwrap();
	assert_eq!(
		(reference.channel_id.as_str(), reference.message_id.as_str()),
		("10", "90")
	);
	assert_eq!(reference.guild_id, None);
	assert_eq!(
		(action.plugin, action.action),
		(plugin.plugin.clone(), plugin.action.clone())
	);
	assert_eq!(text, "Synthetic result to revisit");
	assert_eq!(
		(target.channel, target.message, target.author),
		(Id(10), Id(90), test_support::message(90, Id(10)).author.id)
	);
}

/// Checks that a primary click on a result never opens its menu, and that a result shown only
/// as the preview-limit notice offers its link and ID but no text copy or plugin actions.
#[test]
fn result_menu_withholds_text_actions_for_the_preview_notice() {
	let ctx = egui::Context::default();
	let mut state = state();
	let mut view = MessagingUi::default();
	view.search
		.open_extension(Id(10), false, Some("needle".into()));
	state.request_search("needle".into(), None).unwrap();
	page_with(&mut state, 1, model::PREVIEW_LIMIT_NOTICE);
	view.search.extension_actions = std::sync::Arc::new(vec![crate::extensions_ui::MenuAction {
		plugin: "synthetic.words".into(),
		action: "count".into(),
		label: "Synthetic · Count words".into(),
	}]);
	let mut labels = Vec::new();
	for _ in 0..3 {
		labels = pane_frame(&ctx, &mut view, &mut state, vec![]).0;
	}
	let result = labels
		.iter()
		.find(|(text, _)| text == model::PREVIEW_LIMIT_NOTICE)
		.expect("notice is drawn")
		.1;
	let card = egui::pos2(result.right() + 8.0, result.center().y);
	let press = |button| {
		let mut events = vec![egui::Event::PointerMoved(card)];
		events.extend([true, false].map(|pressed| egui::Event::PointerButton {
			pos: card,
			button,
			pressed,
			modifiers: egui::Modifiers::NONE,
		}));
		events
	};
	let id_label = crate::i18n::translate("timeline-message-actions-copy-message-id");
	let has = |labels: &[(String, egui::Rect)], key: &str| {
		let label = crate::i18n::translate(key);
		labels.iter().any(|(text, _)| *text == label)
	};
	pane_frame(
		&ctx,
		&mut view,
		&mut state,
		press(egui::PointerButton::Primary),
	);
	let labels = pane_frame(&ctx, &mut view, &mut state, vec![]).0;
	assert!(
		!labels.iter().any(|(text, _)| *text == id_label),
		"a primary click does not open the menu"
	);
	pane_frame(
		&ctx,
		&mut view,
		&mut state,
		press(egui::PointerButton::Secondary),
	);
	let labels = pane_frame(&ctx, &mut view, &mut state, vec![]).0;
	assert!(has(&labels, "timeline-message-actions-copy-message-id"));
	assert!(has(&labels, "timeline-message-actions-copy-message-link"));
	assert!(
		!has(&labels, "message-menu-copy"),
		"the notice is not copied as the message"
	);
	assert!(
		!has(&labels, "timeline-message-actions-extensions"),
		"plugins never receive the notice as message text"
	);
}

/// Checks that a text selection dragged in a result counts as that result's selection, which
/// its menu may copy, and not as a selection of any other item.
#[test]
fn a_selection_belongs_to_the_result_it_was_made_in() {
	let ctx = egui::Context::default();
	// The client's style installs the selection pointer that records where a range began.
	crate::design::apply(&ctx);
	let mut state = state();
	let mut view = MessagingUi::default();
	view.search
		.open_extension(Id(10), false, Some("needle".into()));
	state.request_search("needle".into(), None).unwrap();
	page(&mut state, 1);
	let mut labels = Vec::new();
	for _ in 0..3 {
		labels = pane_frame(&ctx, &mut view, &mut state, vec![]).0;
	}
	let text = labels
		.iter()
		.find(|(text, _)| text == "Synthetic result to revisit")
		.expect("result is drawn")
		.1;
	let (start, end) = (
		egui::pos2(text.left() + 1.0, text.center().y),
		egui::pos2(text.right() - 1.0, text.center().y),
	);
	let button = |at: egui::Pos2, button, pressed| egui::Event::PointerButton {
		pos: at,
		button,
		pressed,
		modifiers: egui::Modifiers::NONE,
	};
	for events in [
		vec![
			egui::Event::PointerMoved(start),
			button(start, egui::PointerButton::Primary, true),
		],
		vec![egui::Event::PointerMoved(text.center())],
		vec![egui::Event::PointerMoved(end)],
		vec![button(end, egui::PointerButton::Primary, false)],
	] {
		pane_frame(&ctx, &mut view, &mut state, events);
	}
	assert!(
		crate::select::has_selection(&ctx),
		"the result text is selected"
	);
	assert!(crate::select::has_selection_in(&ctx, text.expand(4.0)));
	let elsewhere = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(4.0, 4.0));
	assert!(
		!crate::select::has_selection_in(&ctx, elsewhere),
		"the selection does not belong to another item"
	);
}
