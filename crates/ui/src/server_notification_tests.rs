//! Synthetic input checks for account notification controls; no service requests are sent.
use crate::{MessagingUi, design};
use client_core::{Command, Envelope, Event, State};
use egui::{Pos2, Rect};
use model::{Id, Patch};

struct Harness {
	ctx: egui::Context,
	view: MessagingUi,
	state: State,
	commands: Vec<Command>,
	size: egui::Vec2,
	rail: bool,
}

fn labels(shape: &egui::Shape, clip: Rect, output: &mut Vec<(String, Rect)>) {
	match shape {
		egui::Shape::Text(text) => {
			let rect = text.visual_bounding_rect();
			if clip.intersects(rect) {
				output.push((text.galley.job.text.clone(), rect));
			}
		}
		egui::Shape::Vec(shapes) => {
			for shape in shapes {
				labels(shape, clip, output);
			}
		}
		_ => {}
	}
}

impl Harness {
	fn new(width: f32, height: f32, light: bool) -> Self {
		let ctx = egui::Context::default();
		design::apply(&ctx);
		ctx.set_visuals(if light {
			egui::Visuals::light()
		} else {
			egui::Visuals::dark()
		});
		let mut state = test_support::chat_demo_state();
		let mut snapshot = test_support::permission_snapshot(&state);
		for guild in &mut snapshot.guilds {
			guild.owner = Some(Id(u64::MAX));
			for role in guild.roles.iter_mut().flatten() {
				role.bits = model::permissions::VIEW_CHANNEL;
			}
		}
		state.apply(Envelope {
			generation: state.generation,
			event: Event::Permissions(client_core::permissions::Event::Snapshot(snapshot)),
		});
		assert!(!state.can_manage_guild(Id(10)));
		state.guild_folders = Some(model::guild_folders::Settings {
			folders: vec![model::guild_folders::Folder {
				guild_ids: vec![Id(10)],
				..Default::default()
			}],
			..Default::default()
		});
		state.folders_pending = false;
		state.folders_stale = false;
		Self {
			ctx,
			state,
			commands: vec![],
			size: egui::vec2(width, height),
			rail: false,
			view: MessagingUi {
				guild: Some(Id(10)),
				..Default::default()
			},
		}
	}

	fn frame(&mut self, events: Vec<egui::Event>) -> Vec<(String, Rect)> {
		let mut text = vec![];
		let output = self.ctx.run_ui(
			egui::RawInput {
				screen_rect: Some(Rect::from_min_size(Pos2::ZERO, self.size)),
				events,
				focused: true,
				..Default::default()
			},
			|ui| {
				if self.rail {
					self.view
						.server_folders(ui, &mut self.state, &mut self.commands);
				} else {
					self.view
						.server_menu
						.header(ui, &mut self.state, Id(10), "Synthetic server");
				}
				self.view.server_menu.show(
					ui.ctx(),
					&mut self.state,
					self.view.guild,
					&mut self.commands,
					&mut self.view.avatars,
				);
			},
		);
		for shape in &output.shapes {
			labels(&shape.shape, shape.clip_rect, &mut text);
		}
		output.drop_without_applying_deltas();
		text
	}

	fn settle(&mut self) -> Vec<(String, Rect)> {
		self.frame(vec![]);
		self.frame(vec![])
	}

	fn point(&mut self, label: &str) -> Pos2 {
		let text = self.settle();
		text.iter()
			.rev()
			.find(|(text, _)| text == label)
			.unwrap_or_else(|| panic!("missing visible control {label}: {text:?}"))
			.1
			.center()
	}

	fn click_at(&mut self, point: Pos2, button: egui::PointerButton) {
		for pressed in [true, false] {
			self.frame(vec![
				egui::Event::PointerMoved(point),
				egui::Event::PointerButton {
					pos: point,
					button,
					pressed,
					modifiers: egui::Modifiers::NONE,
				},
			]);
		}
	}

	fn click(&mut self, label: &str) {
		let point = self.point(label);
		self.click_at(point, egui::PointerButton::Primary);
	}
	fn key(&mut self, key: egui::Key) {
		for pressed in [true, false] {
			self.frame(vec![egui::Event::Key {
				key,
				physical_key: None,
				pressed,
				repeat: false,
				modifiers: egui::Modifiers::NONE,
			}]);
		}
		self.settle();
	}

	fn open(&mut self) {
		self.view
			.server_menu
			.open_notifications(&mut self.state, Id(10));
		self.settle();
	}
	fn open_menu(&mut self) {
		if self.rail {
			self.settle();
			self.click_at(egui::pos2(24.0, 24.0), egui::PointerButton::Secondary);
		} else {
			self.click("Synthetic server");
		}
	}

	fn write(&mut self) -> Box<client_core::preferences_sync::Request> {
		self.state.flush_channel_preferences();
		let Some(Command::ChannelPreferences(request)) = self
			.state
			.poll_channel_preferences(std::time::Instant::now())
		else {
			panic!("queued account preference write")
		};
		request
	}
	fn acknowledge(&mut self, request: &client_core::preferences_sync::Request) {
		let event = self.state.demo_channel_preferences(request);
		self.state.apply(Envelope {
			generation: self.state.generation,
			event: Event::ChannelPreferences(event),
		});
	}
	fn shown(&mut self) -> bool {
		self.settle()
			.iter()
			.any(|(text, _)| text == "Use server default")
	}
}

#[test]
fn ordinary_members_open_notification_settings_from_header_and_rail() {
	for rail in [false, true] {
		let mut h = Harness::new(800.0, 800.0, false);
		h.rail = rail;
		if rail {
			// The isolated rail begins at the content origin with one 48px guild avatar.
			h.view.guild = None;
			h.settle();
			h.click_at(egui::pos2(24.0, 24.0), egui::PointerButton::Secondary);
		} else {
			h.click("Synthetic server");
		}
		let text = h.settle();
		assert!(!text.iter().any(|(text, _)| text == "Server Settings"));
		h.click("Notification Settings");
		assert!(h.shown());
		assert_eq!(h.view.guild, Some(Id(10)));
		assert!(
			h.commands.is_empty(),
			"opening account settings performs no write"
		);
	}
}

#[test]
fn notification_edits_apply_immediately_and_coalesce_one_sparse_write() {
	let mut h = Harness::new(900.0, 900.0, false);
	let original = h.state.server_notification_settings(Id(10));
	h.open();
	let before = std::time::Instant::now();
	h.click("Only mentions");
	h.click("Suppress role mentions");
	assert!(h.commands.is_empty());
	assert_eq!(h.state.server_notification_settings(Id(10)).level, Some(1));
	assert_eq!(
		h.state.server_notification_settings(Id(10)).suppress_roles,
		Some(!original.suppress_roles.unwrap_or(false))
	);
	assert!(h.state.poll_channel_preferences(before).is_none());
	let first = h.write();
	assert_eq!(first.guild, Id(10));
	assert_eq!(first.patch.fields.level, Patch::Value(1));
	assert_eq!(
		first.patch.fields.suppress_roles,
		Patch::Value(!original.suppress_roles.unwrap_or(false))
	);
	assert_eq!(first.patch.fields.muted, Patch::Absent);
	assert!(first.patch.channels.is_empty());
	h.click("All messages");
	assert_eq!(h.state.server_notification_settings(Id(10)).level, Some(0));
	assert!(
		h.state
			.poll_channel_preferences(std::time::Instant::now() + std::time::Duration::from_secs(1))
			.is_none(),
		"one serialized write remains active while controls accept a later choice"
	);
	h.acknowledge(&first);
	assert_eq!(h.state.server_notification_settings(Id(10)).level, Some(0));
	let next = h.write();
	assert_eq!(next.patch.fields.level, Patch::Value(0));
	assert_eq!(next.patch.fields.suppress_roles, Patch::Absent);
	h.acknowledge(&next);
	h.settle();
	assert_eq!(h.state.channel_preference_status(Id(10), None), None);
	assert!(
		h.state
			.poll_channel_preferences(std::time::Instant::now() + std::time::Duration::from_secs(1))
			.is_none()
	);
	assert!(!h.settle().iter().any(|(label, _)| label == "Save changes"));
}

#[test]
fn closing_or_changing_view_preserves_authorized_saves_and_removing_the_server_cancels_unsent_work()
{
	for reason in 0..3 {
		let mut h = Harness::new(900.0, 900.0, false);
		h.open();
		h.click("Nothing");
		assert_eq!(h.state.server_notification_settings(Id(10)).level, Some(2));
		match reason {
			0 => h.click("Close"),
			1 => h.view.guild = None,
			_ => h.state.apply(Envelope {
				generation: h.state.generation,
				event: Event::GuildRemoved(Id(10)),
			}),
		}
		assert!(!h.shown());
		assert!(h.commands.is_empty());
		if reason == 2 {
			assert!(
				h.state
					.poll_channel_preferences(
						std::time::Instant::now() + std::time::Duration::from_secs(1)
					)
					.is_none()
			);
			continue;
		}
		let request = h.write();
		assert_eq!(request.patch.fields.level, Patch::Value(2));
		h.acknowledge(&request);
		h.view.guild = Some(Id(10));
		h.open();
		assert_eq!(h.state.server_notification_settings(Id(10)).level, Some(2));
		assert!(
			h.state
				.poll_channel_preferences(
					std::time::Instant::now() + std::time::Duration::from_secs(1)
				)
				.is_none()
		);
	}
}

#[test]
fn an_old_notification_completion_cannot_cross_logout_generation() {
	let mut h = Harness::new(900.0, 900.0, false);
	h.open();
	h.click("Nothing");
	let request = h.write();
	let generation = request.generation;
	h.state.logout();
	assert!(!h.shown());
	let completion = h.state.demo_channel_preferences(&request);
	h.state.apply(Envelope {
		generation,
		event: Event::ChannelPreferences(completion),
	});
	assert!(!h.state.channel_preferences_known(Id(10)));
	assert!(
		h.state
			.poll_channel_preferences(std::time::Instant::now() + std::time::Duration::from_secs(1))
			.is_none()
	);
}

#[test]
fn ordinary_member_can_choose_a_timed_mute_without_changing_other_fields() {
	for keyboard in [false, true] {
		let mut h = Harness::new(800.0, 850.0, false);
		h.open();
		h.click("Mute server");
		assert_eq!(
			h.state.server_notification_settings(Id(10)).muted,
			Some(true)
		);
		h.click("Until I Turn It Back On");
		if keyboard {
			h.key(egui::Key::ArrowDown);
			h.key(egui::Key::Enter);
		} else {
			h.click("For 15 Minutes");
		}
		assert!(!egui::Popup::is_any_open(&h.ctx));
		let request = h.write();
		assert_eq!(request.guild, Id(10));
		assert_eq!(request.patch.fields.muted, Patch::Value(true));
		let now = std::time::SystemTime::now()
			.duration_since(std::time::UNIX_EPOCH)
			.unwrap()
			.as_secs() as i64;
		assert!(
			matches!(request.patch.fields.mute_until,Patch::Value(until) if (now+895..=now+900).contains(&until))
		);
		assert_eq!(request.patch.fields.level, Patch::Absent);
		assert_eq!(request.patch.fields.suppress_everyone, Patch::Absent);
		assert!(!h.state.can_manage_guild(Id(10)));
	}
}

#[test]
fn quick_server_mute_requires_a_known_baseline_and_offers_an_explicit_refresh() {
	for rail in [false, true] {
		for partial in [false, true] {
			let mut h = Harness::new(800.0, 1000.0, false);
			h.rail = rail;
			h.state.demo = false;
			h.state.auth = client_core::auth::AuthState::Authenticated;
			h.state.gateway_connected = true;
			h.state.notification_preferences = Default::default();
			if partial {
				h.state
					.apply_notification_preferences(client_core::notifications::Event::Settings {
						entries: vec![client_core::notifications::Setting {
							guild: Some(Id(10)),
							level: Some(3),
							..Default::default()
						}],
						replace: false,
					})
					.unwrap();
			}
			assert!(h.state.can_update_server_notifications(Id(10)));
			assert_eq!(h.state.server_notification_settings(Id(10)).muted, None);
			h.open_menu();
			h.click("Mute Server");
			assert!(
				!h.settle()
					.iter()
					.any(|(label, _)| label == "For 15 Minutes"),
				"unloaded preferences must not offer an action the queue rejects"
			);
			assert!(h.commands.is_empty());
			assert!(
				h.state
					.poll_channel_preferences(std::time::Instant::now())
					.is_none()
			);
			h.open_menu();
			h.click("Refresh");
			let [Command::ChannelPreferences(refresh)] = h.commands.as_slice() else {
				panic!("one explicit preference refresh")
			};
			assert_eq!(refresh.guild, Id(10));
			assert_eq!(refresh.patch, model::guild_preferences::Write::default());
		}
	}
}

#[test]
fn quick_server_mute_queues_a_sparse_save_for_an_ordinary_member() {
	for rail in [false, true] {
		for partial in [false, true] {
			let mut h = Harness::new(800.0, 1000.0, false);
			h.rail = rail;
			if partial {
				h.state.notification_preferences = Default::default();
				h.state
					.apply_notification_preferences(client_core::notifications::Event::Settings {
						entries: vec![client_core::notifications::Setting {
							guild: Some(Id(10)),
							muted: Some(false),
							..Default::default()
						}],
						replace: false,
					})
					.unwrap();
			}
			h.open_menu();
			h.click("Mute Server");
			h.click("For 15 Minutes");
			assert!(h.commands.is_empty());
			assert_eq!(
				h.state.server_notification_settings(Id(10)).muted,
				Some(true)
			);
			let request = h.write();
			assert_eq!(request.patch.fields.muted, Patch::Value(true));
			assert!(matches!(request.patch.fields.mute_until, Patch::Value(_)));
			assert_eq!(request.patch.fields.flags, Patch::Absent);
			assert_eq!(request.patch.fields.level, Patch::Absent);
			assert_eq!(request.patch.fields.suppress_everyone, Patch::Absent);
			assert_eq!(request.patch.fields.suppress_roles, Patch::Absent);
			assert!(request.patch.channels.is_empty());
			assert!(!h.state.can_manage_guild(Id(10)));
		}
	}
}

#[test]
fn quick_server_mute_keeps_refresh_loading_and_failure_visible() {
	let mut h = Harness::new(800.0, 1000.0, false);
	h.state.demo = false;
	h.state.auth = client_core::auth::AuthState::Authenticated;
	h.state.gateway_connected = true;
	h.state.notification_preferences = Default::default();
	h.open_menu();
	h.click("Refresh");
	let [Command::ChannelPreferences(refresh)] = h.commands.as_slice() else {
		panic!("one explicit refresh")
	};
	let refresh = refresh.clone();
	h.commands.clear();
	h.open_menu();
	assert!(h.settle().iter().any(|(label, _)| label == "Loading…"));
	h.click("Refresh");
	assert!(
		h.commands.is_empty(),
		"an active refresh cannot be duplicated"
	);
	h.state.apply(Envelope {
		generation: h.state.generation,
		event: Event::ChannelPreferences(client_core::preferences_sync::Finished {
			generation: refresh.generation,
			guild: refresh.guild,
			request: refresh.request,
			result: Err(client_core::auth::Failure::Network),
		}),
	});
	h.open_menu();
	assert!(h.settle().iter().any(|(label, _)| label == "Not confirmed"));
	h.click("Mute Server");
	assert!(
		!h.settle()
			.iter()
			.any(|(label, _)| label == "For 15 Minutes")
	);
	h.open_menu();
	h.click("Refresh");
	assert!(matches!(
		h.commands.as_slice(),
		[Command::ChannelPreferences(_)]
	));
}

#[test]
fn quick_server_mute_rechecks_connection_and_account_before_the_click_finishes() {
	for logout in [false, true] {
		let mut h = Harness::new(800.0, 1000.0, false);
		h.state.demo = false;
		h.state.auth = client_core::auth::AuthState::Authenticated;
		h.state.gateway_connected = true;
		h.open_menu();
		h.click("Mute Server");
		let point = h.point("For 15 Minutes");
		h.frame(vec![
			egui::Event::PointerMoved(point),
			egui::Event::PointerButton {
				pos: point,
				button: egui::PointerButton::Primary,
				pressed: true,
				modifiers: egui::Modifiers::NONE,
			},
		]);
		if logout {
			h.state.logout();
		} else {
			h.state.gateway_connected = false;
		}
		h.frame(vec![egui::Event::PointerButton {
			pos: point,
			button: egui::PointerButton::Primary,
			pressed: false,
			modifiers: egui::Modifiers::NONE,
		}]);
		assert!(h.commands.is_empty());
		h.state.gateway_connected = true;
		assert!(
			h.state
				.poll_channel_preferences(std::time::Instant::now())
				.is_none()
		);
	}
}

#[test]
fn an_uncertain_notification_save_requires_an_explicit_refresh() {
	let mut h = Harness::new(900.0, 900.0, false);
	h.open();
	h.click("Only mentions");
	let request = h.write();
	h.state.apply(Envelope {
		generation: h.state.generation,
		event: Event::ChannelPreferences(client_core::preferences_sync::Finished {
			generation: request.generation,
			guild: request.guild,
			request: request.request,
			result: Err(client_core::auth::Failure::Network),
		}),
	});
	assert!(h.settle().iter().any(|(label, _)| label == "Not confirmed"));
	assert!(h.commands.is_empty());
	assert!(
		h.state
			.poll_channel_preferences(
				std::time::Instant::now() + std::time::Duration::from_secs(60)
			)
			.is_none()
	);
	h.click("Refresh");
	let [Command::ChannelPreferences(refresh)] = h.commands.as_slice() else {
		panic!("one explicit refresh")
	};
	assert_eq!(refresh.guild, Id(10));
	assert_eq!(refresh.patch, model::guild_preferences::Write::default());
}

#[test]
fn notification_modal_can_edit_channel_overrides_without_management_permissions() {
	let mut h = Harness::new(800.0, 850.0, false);
	h.open();
	h.click("Select a channel or category");
	h.click("getting-started");
	assert!(!egui::Popup::is_any_open(&h.ctx));
	assert!(h.commands.is_empty(), "selecting a channel never writes");
	h.click("Nothing");
	assert_eq!(h.state.channel_notification_level(Id(20)), Some(2));
	assert!(h.commands.is_empty());
	let request = h.write();
	assert_eq!(request.guild, Id(10));
	assert_eq!(request.patch.channels.len(), 1);
	assert_eq!(request.patch.channels[0].channel, Id(20));
	assert_eq!(request.patch.channels[0].fields.level, Patch::Value(2));
	assert_eq!(
		request.patch.fields,
		model::guild_preferences::Fields::default()
	);
	assert!(!h.state.can_manage_channel(Id(20)));
	assert!(h.shown());
}

#[test]
fn narrow_notification_dialog_keeps_close_visible_and_scrolls_controls() {
	for light in [false, true] {
		let mut h = Harness::new(320.0, 550.0, light);
		h.open();
		let screen = Rect::from_min_size(Pos2::ZERO, h.size);
		for label in ["Notification Settings", "Use server default", "Close"] {
			assert!(
				screen.contains(h.point(label)),
				"{label} must stay in the viewport"
			);
		}
		for (text, rect) in h.settle() {
			assert!(
				rect.left() >= -1.0 && rect.right() <= h.size.x + 1.0,
				"horizontal overflow: {text}: {rect:?}"
			);
		}
		let content = h.point("Only mentions");
		for _ in 0..4 {
			h.frame(vec![
				egui::Event::PointerMoved(content),
				egui::Event::MouseWheel {
					phase: egui::TouchPhase::Move,
					source: egui::MouseWheelSource::Unknown,
					unit: egui::MouseWheelUnit::Point,
					delta: egui::vec2(0.0, -160.0),
					modifiers: egui::Modifiers::NONE,
				},
			]);
		}
		let roles = h.point("Suppress role mentions");
		assert!(screen.contains(roles));
		h.click_at(roles, egui::PointerButton::Primary);
		assert!(h.commands.is_empty());
		let request = h.write();
		assert!(matches!(
			request.patch.fields.suppress_roles,
			Patch::Value(_)
		));
		assert_eq!(request.patch.fields.level, Patch::Absent);
		assert_eq!(request.patch.fields.muted, Patch::Absent);
		assert_eq!(request.patch.fields.suppress_everyone, Patch::Absent);
		assert_eq!(request.patch.fields.mute_until, Patch::Absent);
	}
}
