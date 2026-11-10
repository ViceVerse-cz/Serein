//! Placement fields: composer toggles and draft-only buttons.
use extensions::{Action, Capability, ExtensionKind, Manifest, Surface};

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
