//! Synthetic input checks for account notification controls; no service requests are sent.
use crate::{MessagingUi, design};
use client_core::{
	Command, Envelope, Event, State,
	server_actions::{Action, NotificationOptions},
};
use egui::{Pos2, Rect};
use model::Id;

struct Harness {
	ctx: egui::Context,
	view: MessagingUi,
	state: State,
	commands: Vec<Command>,
	size: egui::Vec2,
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
				self.view
					.server_menu
					.header(ui, &mut self.state, Id(10), "Synthetic server");
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

	fn open(&mut self) {
		self.view
			.server_menu
			.open_notifications(&mut self.state, Id(10));
		self.settle();
	}

	fn shown(&mut self) -> bool {
		self.settle()
			.iter()
			.any(|(text, _)| text == "Use server default")
	}
}

#[test]
fn notification_edits_require_save_and_submit_one_sparse_write() {
	let mut h = Harness::new(900.0, 900.0, false);
	let original = h.state.server_notification_settings(Id(10));
	h.open();
	h.click("Only mentions");
	h.click("Suppress role mentions");
	assert!(h.commands.is_empty());
	assert_eq!(h.state.server_notification_settings(Id(10)), original);
	h.click("Save changes");
	assert_eq!(h.commands.len(), 1);
	let Command::ServerAction { action, request } = h.commands[0] else {
		panic!("expected account settings write");
	};
	assert_eq!(
		action,
		Action::Notifications {
			guild: Id(10),
			options: NotificationOptions {
				level: Some(1),
				suppress_roles: Some(!original.suppress_roles.unwrap_or(false)),
				..Default::default()
			},
		}
	);
	assert!(h.state.server_action_pending());
	h.click("Saving…");
	h.click("All messages");
	assert_eq!(
		h.commands.len(),
		1,
		"pending controls cannot enqueue a second write"
	);
	h.state.apply(Envelope {
		generation: h.state.generation,
		event: Event::ServerAction(client_core::server_actions::Event::Written {
			action,
			request,
			result: Ok(None),
		}),
	});
	h.settle();
	assert!(!h.state.server_action_pending());
	assert_eq!(h.state.server_notification_settings(Id(10)).level, Some(1));
	h.click("Save changes");
	assert_eq!(h.commands.len(), 1, "acknowledged draft is clean");
}

#[test]
fn closing_or_changing_context_discards_unsaved_notification_edits() {
	for reason in 0..4 {
		let mut h = Harness::new(900.0, 900.0, false);
		h.open();
		h.click("Nothing");
		match reason {
			0 => h.click("Close"),
			1 => {
				h.state.generation += 1;
			}
			2 => {
				h.view.guild = None;
			}
			_ => {
				h.state.guilds.retain(|guild| guild.id != Id(10));
			}
		}
		assert!(!h.shown());
		assert!(h.commands.is_empty());
		if reason != 3 {
			h.view.guild = Some(Id(10));
			h.open();
			h.click("Save changes");
			assert!(
				h.commands.is_empty(),
				"reopening must not save an old draft"
			);
		}
	}
}
