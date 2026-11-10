//! In-conversation outputs: each needs its capability and its surface.
use extensions::{
	Action, Capability, ExtensionKind, Invocation, LocalMessage, Manifest, MessageAlternative,
	Output, Surface,
};

fn action(id: &str, surface: Surface) -> Action {
	Action {
		id: id.into(),
		label: id.into(),
		surface,
		icon: None,
		placement: None,
		menu: None,
		toggle: false,
		icon_on: None,
		needs_draft: false,
	}
}

fn manifest(capabilities: Vec<Capability>, actions: Vec<Action>) -> Manifest {
	Manifest {
		network_hosts: Vec::new(),
		api_version: extensions::API_VERSION,
		id: "chat-check".into(),
		name: "Chat check".into(),
		version: "1.0.0".into(),
		author: "Serein contributors".into(),
		license: "MIT".into(),
		source: "https://example.org/source".into(),
		kind: ExtensionKind::Plugin,
		capabilities,
		actions,
	}
}

fn call(action: &str) -> Invocation {
	Invocation {
		action: action.into(),
		composer: Some("draft".into()),
		..Default::default()
	}
}

fn alternative() -> MessageAlternative {
	MessageAlternative {
		channel_id: "10".into(),
		message_id: "20".into(),
		text: "Hello".into(),
		note: "Translated from Romanian".into(),
		show: true,
	}
}

#[test]
fn toggle_icons_and_draft_only_buttons_belong_to_composer_actions() {
	let mut toggle = action("silent", Surface::Composer);
	toggle.toggle = true;
	toggle.icon_on = Some("<svg/>".into());
	assert!(
		manifest(vec![Capability::Composer], vec![toggle.clone()])
			.validate()
			.is_ok()
	);
	toggle.toggle = false;
	assert!(
		manifest(vec![Capability::Composer], vec![toggle])
			.validate()
			.is_err(),
		"an on-icon needs a toggle"
	);
	let mut preview = action("preview", Surface::Message);
	preview.needs_draft = true;
	assert!(
		manifest(vec![Capability::SelectedMessage], vec![preview])
			.validate()
			.is_err()
	);
}

#[test]
fn alternatives_need_their_capability_and_a_message_surface() {
	let granted = manifest(
		vec![Capability::SelectedMessage, Capability::MessageAlternatives],
		vec![action("translate", Surface::Message)],
	);
	let out = Output {
		alternatives: vec![alternative()],
		..Default::default()
	};
	assert!(out.validate(&granted, &call("translate")).is_ok());
	let ungranted = manifest(
		vec![Capability::SelectedMessage],
		vec![action("translate", Surface::Message)],
	);
	assert!(out.validate(&ungranted, &call("translate")).is_err());
	let too_many = Output {
		alternatives: vec![alternative(); extensions::MAX_ALTERNATIVES + 1],
		..Default::default()
	};
	assert!(too_many.validate(&granted, &call("translate")).is_err());
}

#[test]
fn local_messages_need_their_capability_and_text() {
	let granted = manifest(
		vec![Capability::Composer, Capability::LocalMessages],
		vec![action("preview", Surface::Composer)],
	);
	let local = |content: &str| Output {
		local_message: Some(LocalMessage {
			channel_id: String::new(),
			content: content.into(),
		}),
		..Default::default()
	};
	assert!(local("hello").validate(&granted, &call("preview")).is_ok());
	assert!(local("   ").validate(&granted, &call("preview")).is_err());
	let ungranted = manifest(
		vec![Capability::Composer],
		vec![action("preview", Surface::Composer)],
	);
	assert!(
		local("hello")
			.validate(&ungranted, &call("preview"))
			.is_err()
	);
}
