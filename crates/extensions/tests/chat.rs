//! In-conversation outputs and hooks: each needs its capability and its surface.
use extensions::{
	Action, Capability, Element, ExtensionKind, Hook, Invocation, LocalMessage, Manifest,
	MessageAlternative, Output, Surface,
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
fn hooks_need_their_capabilities_and_appear_once() {
	assert!(
		manifest(
			vec![Capability::Composer],
			vec![action("count", Surface::Draft)]
		)
		.validate()
		.is_ok()
	);
	assert!(
		manifest(vec![], vec![action("count", Surface::Draft)])
			.validate()
			.is_err()
	);
	assert!(
		manifest(
			vec![Capability::Composer],
			vec![action("send", Surface::Send)]
		)
		.validate()
		.is_err()
	);
	assert!(
		manifest(
			vec![Capability::Composer, Capability::SendHooks],
			vec![action("send", Surface::Send)]
		)
		.validate()
		.is_ok()
	);
	assert!(
		manifest(
			vec![Capability::Composer],
			vec![action("a", Surface::Draft), action("b", Surface::Draft)]
		)
		.validate()
		.is_err()
	);
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
fn composer_status_comes_only_from_draft_hooks_and_hooks_never_open_windows() {
	let draft = manifest(
		vec![Capability::Composer, Capability::ComposerStatus],
		vec![action("count", Surface::Draft)],
	);
	let status = Output {
		composer_status: Some("4/2000".into()),
		..Default::default()
	};
	assert!(status.validate(&draft, &call("count")).is_ok());
	let window = Output {
		panel: vec![Element::Text { text: "no".into() }],
		..Default::default()
	};
	assert!(window.validate(&draft, &call("count")).is_err());
	let proposal = Output {
		replacement: Some("x".into()),
		..Default::default()
	};
	assert!(
		proposal.validate(&draft, &call("count")).is_err(),
		"a draft hook cannot rewrite the draft"
	);
	let composer = manifest(
		vec![Capability::Composer, Capability::ComposerStatus],
		vec![action("button", Surface::Composer)],
	);
	assert!(status.validate(&composer, &call("button")).is_err());
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

#[test]
fn send_hooks_answer_with_text() {
	let send = manifest(
		vec![Capability::Composer, Capability::SendHooks],
		vec![action("send", Surface::Send)],
	);
	let mut input = call("send");
	input.hook = Some(Hook::Send);
	assert!(
		Output {
			replacement: Some("Hello".into()),
			..Default::default()
		}
		.validate(&send, &input)
		.is_ok()
	);
}

#[test]
fn hook_inputs_carry_the_draft_and_match_their_surface() {
	let plugin = manifest(
		vec![Capability::Composer, Capability::SendHooks],
		vec![
			action("count", Surface::Draft),
			action("send", Surface::Send),
			action("button", Surface::Composer),
		],
	);
	let mut draft = call("count");
	assert!(
		draft.validate(&plugin).is_err(),
		"a draft action runs only as the draft hook"
	);
	draft.hook = Some(Hook::Draft);
	assert!(draft.validate(&plugin).is_ok());
	let mut send = call("send");
	send.hook = Some(Hook::Edit);
	assert!(send.validate(&plugin).is_ok());
	let mut click = call("button");
	assert!(click.validate(&plugin).is_ok());
	click.hook = Some(Hook::Send);
	assert!(click.validate(&plugin).is_err(), "a click is never a hook");
}
