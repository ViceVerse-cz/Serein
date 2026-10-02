use model::registered_games::RunningGame;
use ui::MessagingUi;

fn frame(
	ctx: &egui::Context,
	view: &mut MessagingUi,
	state: &mut client_core::State,
	events: Vec<egui::Event>,
) -> Vec<(String, egui::Pos2)> {
	let output = ctx.run_ui(
		egui::RawInput {
			screen_rect: Some(egui::Rect::from_min_size(
				egui::Pos2::ZERO,
				egui::vec2(1120.0, 760.0),
			)),
			events,
			..Default::default()
		},
		|ui| {
			view.show(ui, state);
		},
	);
	let texts = output
		.shapes
		.iter()
		.filter_map(|shape| match &shape.shape {
			egui::Shape::Text(text) => Some((
				text.galley.job.text.clone(),
				text.pos + text.galley.size() / 2.0,
			)),
			_ => None,
		})
		.collect();
	output.drop_without_applying_deltas();
	texts
}

fn click(position: egui::Pos2) -> Vec<egui::Event> {
	let button = |pressed| egui::Event::PointerButton {
		pos: position,
		button: egui::PointerButton::Primary,
		pressed,
		modifiers: Default::default(),
	};
	vec![
		egui::Event::PointerMoved(position),
		button(true),
		button(false),
	]
}

fn find(texts: &[(String, egui::Pos2)], label: &str) -> egui::Pos2 {
	texts
		.iter()
		.find(|(text, _)| text == label)
		.unwrap_or_else(|| panic!("{label:?} is not visible"))
		.1
}

#[test]
fn current_game_can_be_renamed_and_running_programs_added() {
	let ctx = egui::Context::default();
	ui::design::apply(&ctx);
	let mut state = test_support::demo_state();
	state.demo = false;
	state.gateway_connected = true;
	let mut view = MessagingUi::default();
	view.share_game_activity = true;
	view.running_game = Some(RunningGame {
		executable: "lms.exe".into(),
		name: "Last Man Standing".into(),
		application: Some(model::Id(7)),
		renamed: false,
	});
	view.preview_settings("registered");
	let mut texts = Vec::new();
	for _ in 0..3 {
		texts = frame(&ctx, &mut view, &mut state, vec![]);
	}
	for label in [
		"Current Game",
		"Now Playing!",
		"Added Games",
		"No games added",
	] {
		find(&texts, label);
	}

	// Rename the detected game inline.
	let name = find(&texts, "Last Man Standing");
	frame(&ctx, &mut view, &mut state, click(name));
	frame(&ctx, &mut view, &mut state, vec![]);
	frame(
		&ctx,
		&mut view,
		&mut state,
		vec![
			egui::Event::Key {
				key: egui::Key::A,
				physical_key: None,
				pressed: true,
				repeat: false,
				modifiers: egui::Modifiers::COMMAND,
			},
			egui::Event::Text("Intel service".into()),
			egui::Event::Key {
				key: egui::Key::Enter,
				physical_key: None,
				pressed: true,
				repeat: false,
				modifiers: Default::default(),
			},
		],
	);
	assert_eq!(view.registered_games.len(), 1);
	assert_eq!(view.registered_games[0].name, "Intel service");
	assert_eq!(view.registered_games[0].application, Some(model::Id(7)));

	// "Add it!" asks for the running programs and adds the chosen one.
	let texts = frame(&ctx, &mut view, &mut state, vec![]);
	frame(&ctx, &mut view, &mut state, click(find(&texts, "Add it!")));
	assert!(view.running_processes_request);
	view.running_processes = Some(vec!["/opt/My Game/mygame.x86_64".into()]);
	let texts = frame(&ctx, &mut view, &mut state, vec![]);
	frame(
		&ctx,
		&mut view,
		&mut state,
		click(find(&texts, "mygame.x86_64")),
	);
	let texts = frame(&ctx, &mut view, &mut state, vec![]);
	frame(&ctx, &mut view, &mut state, click(find(&texts, "Add Game")));
	assert_eq!(view.registered_games.len(), 2);
	assert_eq!(
		view.registered_games[1].executable,
		"opt/my game/mygame.x86_64"
	);
	assert_eq!(view.registered_games[1].application, None);
	let texts = frame(&ctx, &mut view, &mut state, vec![]);
	find(&texts, "Intel service");
	find(&texts, "opt/my game/mygame.x86_64");
}

#[test]
fn one_page_shares_activity_and_lists_detected_games_that_can_be_hidden() {
	let ctx = egui::Context::default();
	ui::design::apply(&ctx);
	let mut state = test_support::demo_state();
	state.demo = false;
	state.gateway_connected = true;
	let mut view = MessagingUi::default();
	view.share_game_activity = true;
	view.discord_activity_sharing = Some(false);
	let now = std::time::SystemTime::now()
		.duration_since(std::time::UNIX_EPOCH)
		.unwrap()
		.as_millis() as u64;
	let game = |executable: &str, name: &str, application: Option<u64>, last_played| {
		model::registered_games::RegisteredGame {
			executable: executable.into(),
			name: name.into(),
			application: application.map(model::Id),
			hidden: false,
			last_played,
		}
	};
	view.registered_games = vec![
		game("opt/manual/game", "Manual game", None, None),
		game("opt/scanned/game", "Scanned game", Some(7), Some(now)),
	];
	// The former Game Activity search term still opens the merged page.
	view.preview_settings("activity");
	let mut texts = Vec::new();
	for _ in 0..3 {
		texts = frame(&ctx, &mut view, &mut state, vec![]);
	}
	for label in [
		"Registered Games",
		"Share game activity",
		"Enable on Discord",
		"Current Game",
		"Added Games",
		"Scanned game",
		"Detected automatically · Last played today",
		"Manual game",
		"opt/manual/game",
	] {
		find(&texts, label);
	}
	assert!(texts.iter().all(|(text, _)| text != "Game Activity"));
	// The most recently played game is listed first.
	assert!(find(&texts, "Scanned game").y < find(&texts, "Manual game").y);

	// A hidden detection stays listed so it can be restored.
	view.registered_games[1].hidden = true;
	let texts = frame(&ctx, &mut view, &mut state, vec![]);
	find(&texts, "Hidden. Serein will not detect this game.");
	frame(&ctx, &mut view, &mut state, click(find(&texts, "Restore")));
	assert!(!view.registered_games[1].hidden);
	assert_eq!(view.registered_games.len(), 2);
}
