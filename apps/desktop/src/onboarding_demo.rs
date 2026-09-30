//! Offline rules screening and onboarding fixture; Discord is never contacted.
use client_core::{
	Envelope, Event, State,
	onboarding::{Action, Event as OnboardingEvent},
};
use model::{
	Id,
	onboarding::{
		Features, Field, FieldKind, Form, GuildGate, MemberGate, Onboarding, Outcome, Prompt,
		PromptOption, Verification,
	},
};

fn option(id: u64, emoji: &str, title: &str, description: &str) -> PromptOption {
	PromptOption {
		id: Id(id),
		title: title.into(),
		description: Some(description.into()),
		emoji: Some(emoji.into()),
	}
}

fn form() -> Form {
	Form {
		verification: Some(Verification {
			version: None,
			description: Some("A calm synthetic workspace for trying Serein offline.".into()),
			fields: vec![Field {
				kind: FieldKind::Terms,
				wire_kind: "TERMS".into(),
				label: "Read and agree to the server rules".into(),
				description: None,
				placeholder: None,
				rules: vec![
					"Be kind and assume good intent.".into(),
					"Keep conversations in the matching channel.".into(),
					"No spam, advertising or unsolicited DMs.".into(),
				],
				choices: Vec::new(),
				required: true,
			}],
		}),
		onboarding: Some(Onboarding {
			prompts: vec![
				Prompt {
					id: Id(9001),
					title: "What brings you here?".into(),
					single_select: false,
					required: true,
					options: vec![
						option(9101, "🎨", "Design", "Mockups, icons and themes"),
						option(9102, "🦀", "Rust", "Native code and performance"),
						option(9103, "🎧", "Hanging out", "Voice chat and music"),
					],
				},
				Prompt {
					id: Id(9002),
					title: "Pick a notification style".into(),
					single_select: true,
					required: false,
					options: vec![
						option(9201, "🔔", "Everything", "Get pinged for announcements"),
						option(9202, "🌙", "Quiet", "Only direct mentions"),
					],
				},
			],
			responses: Vec::new(),
		}),
	}
}

pub fn execute(guild: Id, request: u64, action: Action) -> Event {
	Event::Onboarding(match action {
		Action::Load { .. } => OnboardingEvent::Loaded {
			guild,
			request,
			result: Ok(Box::new(form())),
		},
		Action::Submit { .. } => OnboardingEvent::Submitted {
			guild,
			request,
			result: Ok(Outcome::Approved),
		},
	})
}

/// Marks the first fixture server as freshly joined and still behind its rules.
pub fn open(state: &mut State) {
	let guild = state.guilds[0].id;
	state.apply(Envelope {
		generation: state.generation,
		event: Event::Onboarding(OnboardingEvent::Gates {
			snapshot: false,
			gates: vec![GuildGate {
				guild,
				features: Some(Features {
					verification: true,
					onboarding: true,
				}),
				member: Some(MemberGate {
					pending: true,
					flags: 0,
				}),
			}],
		}),
	});
	if !std::env::args().any(|arg| arg == "--demo-onboarding=closed") {
		state.open_onboarding(guild);
	}
	state.status = "Offline fixture · server onboarding opened at startup";
}
