use client_core::{Command, State, auth::AuthState, interactions};
use model::{Component, ComponentOption, Freshness, Id};

struct Preview {
	ctx: egui::Context,
	view: ui::MessagingUi,
	state: State,
	labels: Vec<(String, egui::Pos2)>,
	commands: Vec<Command>,
	time: f64,
}

impl Preview {
	fn new(component: Component) -> Self {
		let ctx = egui::Context::default();
		ui::design::apply(&ctx);
		let mut state = test_support::demo_state();
		state.auth = AuthState::Authenticated;
		state.gateway_connected = true;
		state.freshness = Freshness::Fresh;
		let mut message = test_support::message(999, state.selected.unwrap());
		message.content = "Synthetic component test".into();
		message.embeds.clear();
		message.application_id = Some(Id(123));
		message.extra_content.components = true;
		message.components = vec![Component {
			kind: 1,
			components: vec![component],
			..Default::default()
		}];
		state.timeline.clear();
		state.timeline.insert(message, true, false).unwrap();
		let mut preview = Self {
			ctx,
			view: Default::default(),
			state,
			labels: vec![],
			commands: vec![],
			time: 0.0,
		};
		preview.settle();
		preview
	}

	fn frame(&mut self, events: Vec<egui::Event>) {
		self.time += 0.1;
		let output = self.ctx.run_ui(
			egui::RawInput {
				screen_rect: Some(egui::Rect::from_min_size(
					egui::Pos2::ZERO,
					egui::vec2(1120.0, 900.0),
				)),
				time: Some(self.time),
				focused: true,
				events,
				..Default::default()
			},
			|ui| self.commands.extend(self.view.show(ui, &mut self.state)),
		);
		fn collect(shape: &egui::Shape, labels: &mut Vec<(String, egui::Pos2)>) {
			match shape {
				egui::Shape::Text(text) => labels.push((
					text.galley.job.text.clone(),
					text.pos + text.galley.size() / 2.0,
				)),
				egui::Shape::Vec(shapes) => {
					for shape in shapes {
						collect(shape, labels);
					}
				}
				_ => {}
			}
		}
		self.labels.clear();
		for shape in &output.shapes {
			collect(&shape.shape, &mut self.labels);
		}
		assert!(output.platform_output.commands.is_empty());
		output.drop_without_applying_deltas();
	}

	fn settle(&mut self) {
		for _ in 0..3 {
			self.frame(vec![]);
		}
	}

	fn click(&mut self, label: &str) {
		let position = self
			.labels
			.iter()
			.find(|(text, _)| text == label)
			.unwrap_or_else(|| panic!("Missing {label:?}: {:?}", self.labels))
			.1;
		for pressed in [true, false] {
			self.frame(vec![
				egui::Event::PointerMoved(position),
				egui::Event::PointerButton {
					pos: position,
					button: egui::PointerButton::Primary,
					pressed,
					modifiers: Default::default(),
				},
			]);
		}
		self.settle();
	}

	fn interaction(&self) -> &interactions::Request {
		self.commands
			.iter()
			.rev()
			.find_map(|command| match command {
				Command::Interaction(request) => Some(request),
				_ => None,
			})
			.expect("Explicit component click produces an interaction")
	}
}

fn selection(required: bool) -> Component {
	Component {
		kind: 3,
		custom_id: Some("choice".into()),
		required,
		min_values: Some(u16::from(required)),
		max_values: Some(1),
		options: ["First", "Second"]
			.into_iter()
			.map(|label| ComponentOption {
				label: label.into(),
				value: label.into(),
				default: label == "First",
				..Default::default()
			})
			.collect(),
		..Default::default()
	}
}

#[test]
fn required_single_select_can_replace_its_default_at_capacity() {
	let mut preview = Preview::new(selection(true));
	assert!(
		!preview
			.labels
			.iter()
			.any(|(text, _)| text.contains("Open in Discord"))
	);
	preview.click("First");
	assert!(
		!preview
			.labels
			.iter()
			.any(|(text, _)| text == "Clear selection")
	);
	preview.click("Second");
	assert!(matches!(&preview.interaction().data,
		interactions::Data::Component { custom_id, values, .. }
		if custom_id == "choice" && values == &["Second"]));
}

#[test]
fn zero_minimum_message_select_can_submit_an_empty_selection() {
	let mut preview = Preview::new(selection(false));
	preview.click("First");
	preview.click("Clear selection");
	assert!(matches!(&preview.interaction().data,
		interactions::Data::Component { values, .. } if values.is_empty()));
}

#[test]
fn optional_modal_select_can_clear_a_default_and_submit() {
	let mut preview = Preview::new(selection(true));
	let Command::Interaction(request) = preview
		.state
		.prepare_component(Id(999), "choice", vec!["First".into()])
		.unwrap()
	else {
		panic!("Expected component request")
	};
	let mut optional = selection(false);
	optional.min_values = None;
	preview
		.state
		.apply_interaction(interactions::Event::Modal {
			nonce: request.nonce,
			modal: Box::new(interactions::Modal {
				id: Id(456),
				application_id: Id(123),
				custom_id: "form".into(),
				title: "Synthetic form".into(),
				components: vec![Component {
					kind: 18,
					label: Some("Optional choice".into()),
					component: Some(Box::new(optional)),
					..Default::default()
				}],
			}),
		})
		.unwrap();
	preview.settle();
	// The modal is painted after the underlying message and owns the final matching label.
	preview.labels.reverse();
	preview.click("First");
	preview.click("Clear selection");
	preview.click("Submit");
	assert!(matches!(&preview.interaction().data,
		interactions::Data::Modal { components, .. }
		if components[0].component.as_ref().unwrap().values.is_empty()));
}
