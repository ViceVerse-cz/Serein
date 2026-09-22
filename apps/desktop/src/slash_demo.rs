//! Synthetic application catalogs and replies. No request leaves the offline demo adapter.
use client_core::{Command, Envelope, Event, State, interactions};
use model::{Id, application_commands as schema};

pub fn catalog(guild: Option<Id>) -> Vec<schema::Command> {
	let mut commands: Vec<schema::Command> = serde_json::from_value(serde_json::json!([
		{
			"id":"99101", "version":"1", "application_id":"99000", "name":"weather",
			"description":"Look up a city forecast.", "contexts":[0,1],
			"options":[
				{"type":3,"name":"city","description":"City to look up.","required":true,"max_length":100},
				{"type":3,"name":"units","description":"Temperature units.","choices":[
					{"name":"Celsius","value":"celsius"},{"name":"Fahrenheit","value":"fahrenheit"}
				]},
				{"type":4,"name":"days","description":"Number of forecast days.","min_value":1,"max_value":7},
				{"type":5,"name":"detailed","description":"Include a detailed forecast."}
			]
		},
		{
			"id":"99102", "version":"1", "application_id":"99000", "name":"ping",
			"description":"Check whether the app is responding.", "contexts":[0,1]
		},
		{
			"id":"99104", "version":"1", "application_id":"99000", "name":"help",
			"description":"Get help with the app's commands.", "contexts":[0,1],
			"options":[
				{"type":3,"name":"input","description":"The command or topic to learn about.","max_length":100}
			]
		},
		{
			"id":"99103", "version":"1", "application_id":"99010", "name":"canvas",
			"description":"Create something with Studio.", "contexts":[0,1],
			"options":[{"type":2,"name":"image","description":"Image tools.","options":[
				{"type":1,"name":"create","description":"Create an image from a prompt.","options":[
					{"type":3,"name":"prompt","description":"Describe the image.","required":true,"max_length":500},
					{"type":3,"name":"style","description":"Visual style.","choices":[
						{"name":"Watercolor","value":"watercolor"},{"name":"Pixel art","value":"pixel"}
					]},
					{"type":10,"name":"scale","description":"Output scale, from 0.5 to 2.","min_value":0.5,"max_value":2}
				]}
			]}]
		}
	])).expect("valid synthetic command schema");
	for command in &mut commands {
		command.guild_id = guild;
		// Original generated fixture artwork; never fetch third-party app icons in demo.
		command.application_icon = Some(format!("{:032x}", command.application_id.0));
		command.application_name = if command.application_id == Id(99000) {
			"Atlas (synthetic)"
		} else {
			"Studio (synthetic)"
		}
		.into();
	}
	assert!(schema::valid_catalog(&commands));
	commands
}

pub fn preview() -> State {
	let mut state = test_support::demo_state();
	state.auth = client_core::auth::AuthState::Authenticated;
	state.gateway_connected = true;
	state.freshness = model::Freshness::Fresh;
	let mut permissions = test_support::permission_snapshot(&state);
	for guild in &mut permissions.guilds {
		for role in guild.roles.iter_mut().flatten() {
			role.bits |= model::permissions::USE_APPLICATION_COMMANDS;
		}
	}
	state.permissions.replace(permissions).unwrap();
	let channel = state.selected.expect("synthetic conversation");
	let Some(Command::ApplicationCommands { guild, request, .. }) =
		state.request_application_commands(channel, false)
	else {
		panic!("synthetic application-command permission");
	};
	state.apply(Envelope {
		generation: state.generation,
		event: Event::ApplicationCommands {
			channel,
			request,
			result: Ok(catalog(guild)),
		},
	});
	state.drafts.insert(channel, "/".into());
	state.status = "Synthetic slash commands — all commands and replies stay offline";
	state
}

pub fn respond(state: &mut State, request: interactions::Request) {
	let interactions::Data::ApplicationCommand { invocation } = &request.data else {
		return;
	};
	let options =
		serde_json::to_string(&invocation.options).expect("validated synthetic arguments");
	let details: String = options.chars().take(1200).collect();
	let invoker = state
		.user
		.as_ref()
		.map(|user| serde_json::json!({"id":user.id.to_string(),"username":user.name}));
	let payload = serde_json::json!({
		"interaction":{"id":(99100 + request.request).to_string(),"type":2,"name":invocation.command.name,"user":invoker},
		"id":(99200 + request.request).to_string(),
		"channel_id":request.channel_id.to_string(),
		"author":{"id":request.application_id.to_string(),"username":invocation.command.application_name,"bot":true},
		"application_id":request.application_id.to_string(), "flags":64,
		"content":format!("**Synthetic /{} response**\nArguments: `{details}`\nNo application was contacted.", invocation.command.name)
	});
	let response = discord_protocol::decode::<discord_protocol::MessageDto>(
		&serde_json::to_vec(&payload).unwrap(),
	)
	.expect("valid synthetic private response")
	.into_model();
	state.apply(Envelope {
		generation: state.generation,
		event: Event::Interaction(interactions::Event::Success {
			nonce: request.nonce,
		}),
	});
	state.apply(Envelope {
		generation: state.generation,
		event: Event::Interaction(interactions::Event::Ephemeral(Box::new(response))),
	});
}

pub fn check() {
	let mut state = preview();
	let channel = state.selected.unwrap();
	assert_eq!(state.application_commands.commands.len(), 4);
	assert_eq!(state.drafts[&channel], "/");
	assert!(
		state
			.prepare_application_command(Id(99101), &[], &[])
			.is_err()
	);
	let values = [
		("city".into(), "Prague".into()),
		("units".into(), "celsius".into()),
		("days".into(), "3".into()),
		("detailed".into(), "true".into()),
	];
	let Command::Interaction(request) = state
		.prepare_application_command(Id(99101), &[], &values)
		.unwrap()
	else {
		panic!("application interaction");
	};
	assert!(request.valid());
	assert!(
		state
			.prepare_application_command(Id(99102), &[], &[])
			.is_err()
	);
	respond(&mut state, request);
	assert!(!state.interactions.busy());
	assert_eq!(state.interactions.ephemeral.len(), 1);
	let response = &state.interactions.ephemeral[0];
	assert!(response.content.contains("Prague") && response.content.contains("celsius"));
	assert!(state.timeline.get(response.id).is_none());
	assert_eq!(
		state.drafts[&channel], "/",
		"invocations do not consume unrelated drafts"
	);
	let path = ["image".into(), "create".into()];
	let values = [
		("prompt".into(), "A moonlit forest".into()),
		("scale".into(), "1.5".into()),
	];
	let Command::Interaction(request) = state
		.prepare_application_command(Id(99103), &path, &values)
		.unwrap()
	else {
		panic!("nested application interaction");
	};
	assert!(request.valid());
	let interactions::Data::ApplicationCommand { invocation } = &request.data else {
		panic!("application invocation");
	};
	assert_eq!(invocation.options[0].name, "image");
	assert_eq!(invocation.options[0].options[0].name, "create");
	respond(&mut state, request);
	assert_eq!(state.interactions.ephemeral.len(), 2);
	println!(
		"Slash command debug check passed: two synthetic apps, nested command, typed options, choices, duplicate prevention, private response and draft isolation."
	);
}
