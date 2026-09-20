//! Offline component -> interaction -> modal -> private response debug exercise.
use client_core::{
	Command, Envelope, Event,
	interactions::{Data, Modal},
};
use model::{Component, Id};

pub fn preview() -> client_core::State {
	let mut state = test_support::demo_state();
	state.gateway_connected = true;
	state.freshness = model::Freshness::Fresh;
	state.auth = client_core::auth::AuthState::Authenticated;
	let payload = serde_json::json!({
		"id":"99001", "channel_id":state.selected.unwrap().to_string(),
		"author":{"id":"99000","username":"Synthetic support","bot":true},
		"application_id":"99000", "content":"Component preview — choose a topic or open the sample form. All interactions stay offline.",
		"components":[
			{"type":1,"components":[{"type":3,"custom_id":"topic","placeholder":"Choose a support topic","options":[
				{"label":"Support","value":"support","description":"Get help with general questions or issues.","emoji":{"id":"9001","name":"support"}},
				{"label":"Purchase / Billing","value":"billing","description":"Get help with purchases and invoices.","emoji":{"id":"9002","name":"billing"}}
			]}]},
			{"type":1,"components":[{"type":2,"style":1,"label":"Open sample form","custom_id":"form"},
				{"type":2,"style":5,"label":"Documentation","url":"https://example.com"}]}
		]
	});
	let message = discord_protocol::decode::<discord_protocol::MessageDto>(
		&serde_json::to_vec(&payload).unwrap(),
	)
	.unwrap()
	.into_model();
	state.timeline.clear();
	state.timeline.insert(message, true, false).unwrap();
	state
}

pub fn respond(request: client_core::interactions::Request) -> Event {
	let event = if matches!(request.data, Data::Modal { .. }) {
		client_core::interactions::Event::Success {
			nonce: request.nonce,
		}
	} else {
		client_core::interactions::Event::Modal {
			nonce: request.nonce,
			modal: Box::new(Modal {
				id: Id(99002),
				application_id: Id(99000),
				custom_id: "sample-form".into(),
				title: "Purchase / Billing".into(),
				components: [
					"Username",
					"Product name",
					"Invoice / order ID",
					"Inquiry reason",
				]
				.into_iter()
				.enumerate()
				.map(|(index, label)| Component {
					kind: 4,
					id: index as u32 + 1,
					custom_id: Some(format!("field-{index}")),
					label: Some(label.into()),
					style: Some(if index == 3 { 2 } else { 1 }),
					required: true,
					max_length: Some(1000),
					..Default::default()
				})
				.collect(),
			}),
		}
	};
	Event::Interaction(event)
}

pub fn check() {
	let mut state = test_support::demo_state();
	let channel = state.selected.unwrap();
	state.gateway_connected = true;
	state.freshness = model::Freshness::Fresh;
	state.auth = client_core::auth::AuthState::Authenticated;
	let payload = serde_json::json!({"id":"99001","channel_id":channel.to_string(),"author":{"id":"99000","username":"Synthetic ticket app","bot":true},"application_id":"99000","content":"Open a synthetic support ticket","components":[{"type":1,"components":[{"type":2,"style":1,"label":"Open ticket","custom_id":"open-ticket"},{"type":2,"style":6,"label":"Upgrade","sku_id":"99003"}]}]});
	let message = discord_protocol::decode::<discord_protocol::MessageDto>(
		&serde_json::to_vec(&payload).unwrap(),
	)
	.unwrap()
	.into_model();
	state.timeline.clear();
	state.timeline.insert(message.clone(), true, false).unwrap();
	let ctx = eframe::egui::Context::default();
	ui::design::apply(&ctx);
	let mut view = ui::MessagingUi::default();
	fn text(shape: &eframe::egui::Shape, found: &mut Vec<(String, eframe::egui::Rect)>) {
		match shape {
			eframe::egui::Shape::Text(t) => found.push((
				t.galley.job.text.clone(),
				t.galley.rect.translate(t.pos.to_vec2()),
			)),
			eframe::egui::Shape::Vec(shapes) => shapes.iter().for_each(|s| text(s, found)),
			_ => {}
		}
	}
	let mut frame = |events| {
		let mut commands = vec![];
		let output = ctx.run_ui(
			eframe::egui::RawInput {
				events,
				screen_rect: Some(eframe::egui::Rect::from_min_size(
					eframe::egui::Pos2::ZERO,
					eframe::egui::vec2(1120.0, 900.0),
				)),
				..Default::default()
			},
			|ui| {
				commands = view.show(ui, &mut state);
			},
		);
		let mut labels = vec![];
		for shape in &output.shapes {
			text(&shape.shape, &mut labels);
		}
		output.drop_without_applying_deltas();
		(commands, labels)
	};
	frame(vec![]);
	let (_, labels) = frame(vec![]);
	let position = labels
		.iter()
		.find(|(label, _)| label == "Open ticket")
		.expect("native component button rendered")
		.1
		.center();
	assert!(
		labels.iter().all(|(label, _)| label != "Upgrade"),
		"premium component button omitted"
	);
	let pointer = |pressed| {
		vec![
			eframe::egui::Event::PointerMoved(position),
			eframe::egui::Event::PointerButton {
				pos: position,
				button: eframe::egui::PointerButton::Primary,
				pressed,
				modifiers: eframe::egui::Modifiers::NONE,
			},
		]
	};
	frame(pointer(true));
	let (commands, _) = frame(pointer(false));
	let request = commands
		.into_iter()
		.find_map(|command| match command {
			Command::Interaction(request) => Some(request),
			_ => None,
		})
		.expect("native component click submits interaction");
	assert!(request.valid());
	assert!(
		state
			.prepare_component(message.id, "open-ticket", vec![])
			.is_none(),
		"duplicate blocked"
	);
	let nonce = request.nonce;
	let source = Component {
		kind: 18,
		id: 1,
		label: Some("Issue".into()),
		component: Some(Box::new(Component {
			kind: 4,
			id: 2,
			custom_id: Some("issue".into()),
			required: true,
			min_length: Some(3),
			..Default::default()
		})),
		..Default::default()
	};
	state.apply(Envelope {
		generation: state.generation,
		event: Event::Interaction(client_core::interactions::Event::Modal {
			nonce,
			modal: Box::new(Modal {
				id: Id(99002),
				application_id: Id(99000),
				custom_id: "ticket-form".into(),
				title: "Open ticket".into(),
				components: vec![source.clone()],
			}),
		}),
	});
	assert!(state.interactions.modal.is_some());
	assert!(
		state
			.submit_interaction_modal(vec![source.clone()])
			.is_none(),
		"required text rejected"
	);
	let mut filled = source;
	filled.component.as_mut().unwrap().value = Some("Synthetic issue".into());
	let Command::Interaction(submit) = state.submit_interaction_modal(vec![filled]).unwrap() else {
		panic!()
	};
	assert!(submit.valid() && matches!(submit.data, Data::Modal { .. }));
	state.apply(Envelope {
		generation: state.generation,
		event: Event::Interaction(client_core::interactions::Event::Success {
			nonce: submit.nonce,
		}),
	});
	assert!(state.interactions.modal.is_none());
	let mut response = message;
	response.id = Id(99003);
	response.ephemeral = true;
	response.flags |= 64;
	response.content = "Ticket created (synthetic)".into();
	state.apply(Envelope {
		generation: state.generation,
		event: Event::Interaction(client_core::interactions::Event::Ephemeral(Box::new(
			response,
		))),
	});
	assert_eq!(state.interactions.ephemeral.len(), 1);
	assert!(
		state.timeline.get(Id(99003)).is_none(),
		"private reply excluded from persisted timeline"
	);
	let mut invalid = payload;
	invalid["components"] =
		serde_json::json!([{"type":1,"components":vec![serde_json::json!({"type":2});41]}]);
	assert!(
		discord_protocol::decode::<discord_protocol::MessageDto>(
			&serde_json::to_vec(&invalid).unwrap()
		)
		.is_err()
	);
	let upload = Component {
		kind: 19,
		file_types: vec![".pdf".into()],
		..Default::default()
	};
	assert!(upload.accepts_file("support.PDF") && !upload.accepts_file("support.exe"));
	state.interactions.reset();
	println!(
		"Components debug check passed: bounded decode, native button click, button request, duplicate guard, required modal validation, modal completion and ephemeral isolation."
	);
}
