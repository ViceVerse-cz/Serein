//! Offline native framebuffer capture; no account, filesystem cache, or network adapters.
use eframe::egui;
#[allow(dead_code)] // The member Guide fixture is used by the full desktop demo.
#[path = "../src/community_admin_demo.rs"]
mod community_admin_demo;
#[path = "../src/community_demo.rs"]
mod community_demo;
#[path = "../src/profile_settings_demo.rs"]
mod profile_settings_demo;

#[path = "../src/inbox_demo.rs"]
mod inbox_demo;
#[path = "../src/server_settings_demo.rs"]
mod server_settings_demo;
#[allow(dead_code)] // The shared fixture's CLI check is called by the desktop binary.
#[path = "../src/slash_demo.rs"]
mod slash_demo;
use std::{
	path::PathBuf,
	sync::{
		Arc,
		atomic::{AtomicBool, Ordering},
	},
	thread::JoinHandle,
	time::{Duration, Instant},
};

struct Preview {
	inbox_demo: inbox_demo::Store,
	mfa_fixture_target: Option<client_core::mfa::Target>,
	interactive: bool,
	metrics: Option<NativeMetrics>,
	timeline_churn: bool,
	smoke: bool,
	messaging: ui::MessagingUi,
	state: client_core::State,
	output: PathBuf,
	thumbnail: bool,
	frames: u8,
	/// Wheel distance and pointer position injected over the first frames, for pages below the fold.
	scroll: Option<(f32, egui::Pos2)>,
	requested: bool,
	screenshot: Option<std::sync::mpsc::Receiver<Arc<egui::ColorImage>>>,
	writer: Option<JoinHandle<Result<(), String>>>,
	saved: Arc<AtomicBool>,
	started: Instant,
}

impl eframe::App for Preview {
	fn persist_egui_memory(&self) -> bool {
		false
	}

	fn raw_input_hook(&mut self, _: &egui::Context, raw_input: &mut egui::RawInput) {
		if let Some((distance, at)) = self.scroll.filter(|_| (1..=3).contains(&self.frames)) {
			raw_input.events.push(egui::Event::PointerMoved(at));
			raw_input.events.push(egui::Event::MouseWheel {
				unit: egui::MouseWheelUnit::Point,
				phase: egui::TouchPhase::Move,
				source: egui::MouseWheelSource::Unknown,
				delta: egui::vec2(0.0, -distance / 3.0),
				modifiers: egui::Modifiers::NONE,
			});
		}
	}

	fn ui(&mut self, ui: &mut egui::Ui, _: &mut eframe::Frame) {
		let ctx = ui.ctx().clone();
		if let Some(recovered) = ui::local_time::begin_frame() {
			self.messaging.time_zone = recovered;
		}
		ui::local_time::set_timezone(&self.messaging.time_zone);
		if let Some(request) = ui::local_time::take_timezone_request() {
			let wake = ctx.clone();
			std::thread::spawn(move || {
				request.resolve();
				wake.request_repaint();
			});
		}
		if self.timeline_churn {
			self.state.revision = self.state.revision.wrapping_add(1);
			ctx.request_repaint_after(Duration::from_secs_f64(1.0 / 60.0));
		}
		ui::design::paint_backdrop(&ctx);
		// Only synthetic fixtures execute these commands; no service adapters exist here.
		let wheel_events = ctx.input(|input| {
			input
				.raw
				.events
				.iter()
				.filter(|event| matches!(event, egui::Event::MouseWheel { .. }))
				.count()
		});
		let measured = Instant::now();
		let mut commands = self.messaging.show(ui, &mut self.state);
		commands.extend(self.state.poll_channel_preferences(Instant::now()));
		if let Some(deadline) = self.state.next_channel_preference_deadline() {
			ctx.request_repaint_after(deadline.saturating_duration_since(Instant::now()));
		}
		if let Some(metrics) = &mut self.metrics {
			metrics.record(&ctx, measured.elapsed(), wheel_events);
		}
		for command in commands {
			let event = match command {
				client_core::Command::ChannelPreferences(request) => {
					client_core::Event::ChannelPreferences(
						self.state.demo_channel_preferences(&request),
					)
				}
				command @ (client_core::Command::History { .. }
				| client_core::Command::HistoryAround { .. }
				| client_core::Command::ReplyPreview { .. }
				| client_core::Command::CancelReplyPreviews) => {
					let links = std::env::args().any(|arg| {
						matches!(
							arg.as_str(),
							"--page=channel-links" | "--page=channel-link-replies"
						)
					});
					let Some(event) =
						test_support::offline_history_event(&self.state, &command, links)
					else {
						continue;
					};
					event
				}
				client_core::Command::EditProfile {
					user,
					request,
					changes,
				} => profile_settings_demo::global_profile(&self.state, user, request, changes),
				client_core::Command::ServerProfile {
					user,
					guild,
					request,
					edit,
				} => profile_settings_demo::execute(&self.state, user, guild, request, edit),
				client_core::Command::OwnedCosmetics {
					user,
					guild,
					request,
					change,
					..
				} => profile_settings_demo::owned_cosmetics(
					&self.state,
					user,
					guild,
					request,
					change,
				),

				client_core::Command::Report(request) => test_support::report_demo(*request),
				command @ (client_core::Command::Mentions { .. }
				| client_core::Command::InboxAction { .. }) => self.inbox_demo.execute(&self.state, command),
				client_core::Command::ApplicationCommands {
					channel,
					guild,
					request,
					..
				} => client_core::Event::ApplicationCommands {
					channel,
					request,
					result: Ok(slash_demo::catalog(guild)),
				},
				client_core::Command::Autocomplete(request) => slash_demo::autocomplete(request),
				client_core::Command::Interaction(request) => {
					slash_demo::respond(&mut self.state, request);
					continue;
				}
				client_core::Command::BadgeSettings { request, settings } => {
					client_core::Event::BadgeSettings {
						request,
						result: Ok(settings),
					}
				}
				client_core::Command::Community {
					guild,
					request,
					action,
					..
				} => community_demo::execute(&mut self.state, guild, request, *action),
				client_core::Command::ServerAdmin {
					guild,
					request,
					action,
				} => server_settings_demo::execute_admin(&self.state, guild, request, *action),
				client_core::Command::ServerSettings {
					guild,
					request,
					edit,
				} => server_settings_demo::execute(&self.state, guild, request, edit),
				client_core::Command::ServerAction { action, request } => {
					server_settings_demo::execute_action(&mut self.state, action, request)
				}
				client_core::Command::Mfa(request) => {
					if !self.state.mfa_command_allowed(&request) {
						continue;
					}
					let result = if matches!(request.action, client_core::mfa::Action::Cancel) {
						if let Some(target) = self.mfa_fixture_target.take() {
							self.state.apply(client_core::Envelope {
								generation: self.state.generation,
								event: target.rejected(client_core::auth::Failure::ProtocolAt(
									"Offline preview action cancelled",
								)),
							});
						}
						Ok(client_core::mfa::Status::Cancelled)
					} else {
						Err(client_core::auth::Failure::ProtocolAt(
							"Offline preview: no code is sent and verification is unavailable",
						))
					};
					client_core::Event::Mfa(client_core::mfa::Event::Updated {
						id: request.id,
						result,
					})
				}
				client_core::Command::ChannelAction {
					guild,
					channel,
					request,
					action: client_core::channel_actions::Action::Personal(change),
				} => client_core::Event::ChannelAction(
					client_core::channel_actions::Event::Finished {
						guild,
						channel,
						request,
						result: Ok(client_core::channel_actions::Outcome::Personal(change)),
					},
				),
				client_core::Command::ChannelAction {
					guild,
					channel,
					request,
					action: client_core::channel_actions::Action::Description,
				} => client_core::Event::ChannelAction(
					client_core::channel_actions::Event::Finished {
						guild,
						channel,
						request,
						result: Ok(client_core::channel_actions::Outcome::Description(
							"Synthetic channel description for the community.".into(),
						)),
					},
				),
				client_core::Command::ChannelAction {
					guild,
					channel,
					request,
					action: client_core::channel_actions::Action::Load,
				} => client_core::Event::ChannelAction(
					client_core::channel_actions::Event::Finished {
						guild,
						channel,
						request,
						result: Ok(client_core::channel_actions::Outcome::Details(
							channel_settings(&self.state, channel),
						)),
					},
				),
				_ => continue,
			};
			self.state.apply(client_core::Envelope {
				generation: self.state.generation,
				event,
			});
		}
		for request in std::mem::take(&mut self.messaging.extensions.requests) {
			if let ui::ExtensionRequest::PreviewTheme { theme, image } = request {
				ui::design::set_extension_theme(theme.as_deref());
				ui::design::set_background_image(&ctx, image);
				ui::design::apply(&ctx);
			}
		}
		if self.interactive {
			return;
		}
		if self.smoke {
			self.frames += 1;
			if self.frames >= 5 {
				self.saved.store(true, Ordering::Release);
				println!("Offline UI smoke run completed; no screenshot captured.");
				ctx.send_viewport_cmd(egui::ViewportCommand::Close);
			}
			ctx.request_repaint();
			return;
		}
		if self.requested && self.writer.is_none() {
			let screenshot = self
				.screenshot
				.as_ref()
				.and_then(|receiver| receiver.try_recv().ok());
			if let Some(image) = screenshot {
				let output = self.output.clone();
				let thumbnail = self.thumbnail;
				self.writer = Some(std::thread::spawn(move || {
					if image.size[0] > 4096 || image.size[1] > 4096 {
						return Err("Screenshot exceeds the 4096-pixel dimension limit".into());
					}
					let pixels: Vec<u8> = image
						.pixels
						.iter()
						.flat_map(|pixel| pixel.to_srgba_unmultiplied())
						.collect();
					let image = image::RgbaImage::from_raw(
						image.size[0] as u32,
						image.size[1] as u32,
						pixels,
					)
					.ok_or("Invalid screenshot pixel count")?;
					let image = image::DynamicImage::ImageRgba8(image);
					let image = if thumbnail {
						image.thumbnail(640, 360)
					} else {
						image
					};
					image
						.save_with_format(&output, image::ImageFormat::Png)
						.map_err(|error| error.to_string())
				}));
			}
		}
		if self.writer.as_ref().is_some_and(JoinHandle::is_finished) {
			match self.writer.take().unwrap().join() {
				Ok(Ok(())) => {
					self.saved.store(true, Ordering::Release);
					println!(
						"Saved offline native framebuffer: {}",
						self.output.display()
					);
				}
				Ok(Err(error)) => eprintln!("Screenshot save failed: {error}"),
				Err(_) => eprintln!("Screenshot worker failed"),
			}
			ctx.send_viewport_cmd(egui::ViewportCommand::Close);
			return;
		}
		if self.started.elapsed() > Duration::from_secs(20) {
			eprintln!("Native screenshot callback did not complete within 20 seconds");
			ctx.send_viewport_cmd(egui::ViewportCommand::Close);
			return;
		}
		self.frames = self.frames.saturating_add(1);
		if self.frames >= 5 && self.started.elapsed() >= Duration::from_secs(1) && !self.requested {
			self.requested = true;
			let (send, receive) = std::sync::mpsc::sync_channel(1);
			self.screenshot = Some(receive);
			let wake = ctx.clone();
			ctx.request_screenshot(move |image| {
				let _ = send.try_send(image);
				wake.request_repaint();
			});
		}
		ctx.request_repaint_after(Duration::from_millis(100));
	}
}

fn prime_profile(state: &mut client_core::State) {
	if let Some(client_core::Command::EditProfile { user, request, .. }) = state.load_own_profile()
	{
		let mut profile = ui::synthetic_own_profile(state.user.as_ref().unwrap());
		profile.bio = "✦ quiet corners ✦\nSynthetic preview with a four-pointed star.".into();
		state.apply(client_core::Envelope {
			generation: state.generation,
			event: client_core::Event::ProfileEdited {
				user,
				request,
				result: Ok(Box::new(profile)),
			},
		});
	}
}

/// Several synthetic Rich Presence entries for the first friend and the message author.
fn prime_activities(state: &mut client_core::State) {
	let activity =
		|kind, name: &str, details: Option<&str>, state: Option<&str>| model::RichActivity {
			kind,
			name: name.into(),
			details: details.map(Into::into),
			state: state.map(Into::into),
			image: None,
			small_image: None,
			ends_at: None,
			started_at: Some(1_700_000_000_000),
		};
	let activities = vec![
		activity(
			0,
			"Synthetic Quest",
			Some("Exploring the hollow"),
			Some("Chapter 3"),
		),
		activity(2, "Spotify", Some("Quiet Harbor"), Some("The Offline Band")),
		activity(3, "Harbor Stories", None, None),
	];
	let author = test_support::message(1, model::Id(20)).author.id;
	state.apply(client_core::Envelope {
		generation: state.generation,
		event: client_core::Event::DirectPresence(
			[model::Id(1001), author]
				.into_iter()
				.map(|user| client_core::presence::Update {
					user,
					status: model::Patch::Value("online".into()),
					custom_status: model::Patch::Absent,
					activities: model::Patch::Value(activities.clone()),
					clients: model::Patch::Absent,
				})
				.collect(),
		),
	});
}

fn prime_extension_chat(state: &mut client_core::State) {
	let channel = state.selected.expect("selected fixture channel");
	let messages = [
		"Welcome to our little corner of the internet.",
		"A place for good conversations and late-night ideas.",
		"**Game night** starts at 8. Everyone is welcome!",
		"I'll bring the playlist. Any requests?",
		"Something with a little more synth, please.",
		"Hey everyone, ready for game night?",
	]
	.into_iter()
	.enumerate()
	.map(|(index, content)| {
		let mut message = test_support::message(600 + index as u64, channel);
		message.content = content.into();
		message.attachments.clear();
		message.embeds.clear();
		message.reactions = Some(vec![]);
		message
	})
	.collect();
	state.timeline.clear();
	state
		.timeline
		.seed_cache(messages)
		.expect("valid synthetic conversation");
}

/// Shared before/after fixture; all channel metadata and server artwork are synthetic.
fn prime_channel_links(state: &mut client_core::State) {
	let channel = state.selected.expect("synthetic channel-link conversation");
	let mut guild = state.guilds[0].clone();
	guild.id = model::Id(11);
	guild.name = "Synthetic elsewhere".into();
	guild.icon = Some("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into());
	state.guilds.push(guild);
	let mut foreign = state.channel(channel).unwrap().clone();
	foreign.id = model::Id(30);
	foreign.guild = Some(model::Id(11));
	foreign.parent_id = None;
	foreign.name = "other-server-chat".into();
	state.channels.push(foreign);
	let mut long = state.channel(model::Id(27)).unwrap().clone();
	long.id = model::Id(31);
	long.name = "Příliš žluťoučký kůň · 日本語の長い投稿名 · A deliberately long forum post title for narrow windows".into();
	state.channels.push(long);
	let base = state.channel(model::Id(20)).unwrap().clone();
	for (id, kind, name) in [(32, 5, "announcements"), (33, 0, "staff-only")] {
		let mut extra = base.clone();
		extra.id = model::Id(id);
		extra.kind = kind;
		extra.name = name.into();
		state.channels.push(extra);
	}
	state.invalidate_navigation();
	let mut permissions = test_support::permission_snapshot(state);
	// The cached name of a channel the session cannot view must not be displayed.
	if let Some(hidden) = permissions
		.channels
		.iter_mut()
		.find(|channel| channel.id == model::Id(33))
	{
		hidden.overwrites = Some(vec![model::permissions::Overwrite {
			id: model::Id(10),
			kind: 0,
			allow: 0,
			deny: model::permissions::VIEW_CHANNEL,
		}]);
	}
	state
		.permissions
		.replace(permissions)
		.expect("valid synthetic channel-link permissions");
	state.apply(client_core::Envelope {
		generation: state.generation,
		event: client_core::Event::Permissions(client_core::permissions::Event::Role {
			guild: model::Id(10),
			role: model::permissions::Role {
				icon: None,
				unicode_emoji: None,
				id: model::Id(101),
				name: "Synthetic colored role".into(),
				bits: 0,
				color: 0x68ada4,
				secondary_color: None,
				tertiary_color: None,
				position: 1,
				hoist: false,
			},
		}),
	});
	let mut message = test_support::message(600, channel);
	message.content = "**Channel and thread references**\n\
Regular channel: <#20>\n\
Regular thread: <#28>\n\
Forum channel: <#26>\n\
Forum post: <#27>\n\n\
**Message links**\n\
Regular channel: https://discord.com/channels/10/20/501\n\
Regular thread: https://discord.com/channels/10/28/501\n\
Forum post: https://discord.com/channels/10/27/501\n\
Another server: https://discord.com/channels/11/30/501\n\n\
**Conversations and channel kinds**\n\
Announcement: https://discord.com/channels/10/32/501\n\
Voice chat: https://discord.com/channels/10/25/501\n\
Direct message: https://discord.com/channels/@me/22/501\n\
Group: https://discord.com/channels/@me/29/501\n\
Hidden channel: https://discord.com/channels/10/33/501\n\
Repeated: https://discord.com/channels/10/20/501 https://discord.com/channels/10/20/501\n\
Channel link: https://discord.com/channels/10/32\n\n\
**Mentions and spoilers**\n\
Mentions: <@8001> <@&101> @everyone\n\
Spoilers: ||a hidden synthetic secret|| and ||x||\n\n\
**Fallbacks and long names**\n\
Unavailable: <#999> https://discord.com/channels/10/998/501\n\
Long post: <#31> https://discord.com/channels/10/31/501\n\
Named link: [Open the original message](https://discord.com/channels/10/20/501)\n\
Literal: `<#28>` · Concealed: ||<#27> https://discord.com/channels/11/30/501||"
		.into();
	// Discord's ping-hiding trick: many empty spoilers, then adjacent mentions.
	message.content.push_str(
		"\n\n**How to Join:**\n1. Make your profile themed\n\
2. Take a screenshot, and post it in https://discord.com/channels/10/20\n\n\
**Please read the full rules & information in https://discord.com/channels/10/21**\n",
	);
	message.content.push_str(
		"\n🚀 **How to claim it**\n1. Download it and create an account\n\
3. Copy your **reward code**\n\
4. Redeem it at **[example.com/account/redeem](https://example.com/account/redeem)**\n\n\
The badge unlocks instantly once redeemed.\n",
	);
	message.content.push_str(
		"\n# 🐰 🥚Easter💎Deals🌞Sale\nDon’t 🐣 miss it, it ends soon 🌞.\n\
- **10% off** all gifts\n- **15% off** all products\n",
	);
	message.content.push_str("\nHidden ping: ");
	message.content.push_str(&"||\u{200b}||".repeat(60));
	message.content.push_str(" @everyone<@&101>");
	message.attachments.clear();
	message.embeds.clear();
	message.reactions = Some(vec![]);
	state.timeline.clear();
	state.older_exhausted = true;
	state
		.timeline
		.seed_cache(vec![message])
		.expect("valid synthetic channel-link message");
}

fn prime_channel_link_replies(state: &mut client_core::State) {
	prime_channel_links(state);
	let channel = state.selected.unwrap();
	let mut messages = Vec::new();
	for (index, target) in ["10/20", "10/27", "11/30", "10/31"].iter().enumerate() {
		let mut original = test_support::message(700 + index as u64 * 2, channel);
		original.content = format!("https://discord.com/channels/{target}/501");
		original.attachments.clear();
		original.embeds.clear();
		original.reactions = Some(vec![]);
		let mut reply = original.clone();
		reply.id = model::Id(original.id.0 + 1);
		reply.reply_to = Some(original.id);
		reply.content = "Synthetic reply to the message link above.".into();
		messages.extend([original, reply]);
	}
	state.timeline.clear();
	state.timeline.seed_cache(messages).unwrap();
}

// Fixture packages are checked-in inputs; execution never calls desktop adapters.
fn extension_fixture(
	id: &str,
) -> Result<
	(
		extensions::Package,
		extensions::Invocation,
		Option<extensions::Output>,
	),
	Box<dyn std::error::Error>,
> {
	let external = std::env::var_os("SEREIN_PREVIEW_PACKAGE")
		.map(std::fs::read)
		.transpose()?;
	let bytes: &[u8] = match id {
		"custom-rpc" | "api-proxy" => external
			.as_deref()
			.ok_or("Set SEREIN_PREVIEW_PACKAGE to the external plugin package")?,
		"serein-ocean" => {
			include_bytes!("../../../extensions/themes/ocean.serein-extension")
		}
		"message-delete-protector" => include_bytes!(
			"../../../extensions/plugins/packages/message-delete-protector.serein-extension"
		),
		"serein-midnight" => {
			include_bytes!("../../../extensions/themes/midnight.serein-extension")
		}
		"serein-rose" => {
			include_bytes!("../../../extensions/themes/rose.serein-extension")
		}
		"serein-forest" => {
			include_bytes!("../../../extensions/themes/forest.serein-extension")
		}
		"serein-latte" => {
			include_bytes!("../../../extensions/themes/latte.serein-extension")
		}
		"golden-theme" => {
			include_bytes!("../../../extensions/themes/golden.serein-extension")
		}
		"black-theme" => {
			include_bytes!("../../../extensions/themes/katana.serein-extension")
		}
		"obsidian-theme" => {
			include_bytes!("../../../extensions/themes/obsidian.serein-extension")
		}
		"teal-theme" => {
			include_bytes!("../../../extensions/themes/teal.serein-extension")
		}
		_ => return Err("Unknown fixture extension".into()),
	};
	let package = extensions::parse_package(bytes)?;
	let invocation = extensions::Invocation {
		action: if id == "custom-rpc" {
			"preview"
		} else if id == "api-proxy" {
			"open"
		} else {
			"activate"
		}
		.into(),
		values: if id == "custom-rpc" {
			[
				("application-id", "123456789"),
				("name", "Stargazing"),
				("details", "Exploring the night sky"),
				("state", "In the observatory"),
				("button1-label", "Visit the observatory"),
				("button1-url", "https://example.com/observatory"),
				("party-current", "2"),
				("party-max", "4"),
			]
			.into_iter()
			.map(|(k, v)| (k.into(), v.into()))
			.collect()
		} else {
			Default::default()
		},
		..Default::default()
	};
	let output = if package.theme.is_none() {
		Some(extensions::invoke(&package, &invocation)?)
	} else {
		None
	};
	Ok((package, invocation, output))
}

fn seed_catalog(extensions: &mut ui::ExtensionUi, themes: bool) {
	if themes {
		let packages: [(&[u8], &str); 6] = [
			(
				include_bytes!("../../../extensions/themes/ocean.serein-extension"),
				"",
			),
			(
				include_bytes!("../../../extensions/themes/obsidian.serein-extension"),
				"Obsidian violet surfaces and lavender accents.",
			),
			(
				include_bytes!("../../../extensions/themes/forest.serein-extension"),
				"Calm forest greens and fresh leafy accents.",
			),
			(
				include_bytes!("../../../extensions/themes/latte.serein-extension"),
				"Warm coffee tones and a creamy caramel accent.",
			),
			(
				include_bytes!("../../../extensions/themes/rose.serein-extension"),
				"Soft rose accents.",
			),
			(
				include_bytes!("../../../extensions/themes/midnight.serein-extension"),
				"Deep, quiet surfaces.",
			),
		];
		let mut entries: Vec<_> = packages
			.into_iter()
			.enumerate()
			.map(|(index, (bytes, description))| {
				let package = extensions::parse_package(bytes).expect("valid synthetic theme");
				ui::ExtensionEntry {
					cover_image: None,
					local_theme: index == 0,
					manifest: package.manifest,
					theme_preview: package.theme,
					description: description.into(),
					preview: None,
					reviewed: true,
					sha256: "a".repeat(64),
					download_bytes: bytes.len() as u64,
					enabled: index < 2,
					cleanup_pending: false,
					update_available: false,
					update_manifest: None,
				}
			})
			.collect();
		entries[0].manifest.name = "My ocean".into();
		entries[0].manifest.author = "You".into();
		let image =
			image::load_from_memory(include_bytes!("../../../extensions/previews/ocean.png"))
				.expect("valid synthetic cover")
				.to_rgba8();
		entries[0].cover_image = Some(Arc::new(egui::ColorImage::from_rgba_unmultiplied(
			[image.width() as usize, image.height() as usize],
			image.as_raw(),
		)));
		extensions.active_theme = Some(entries[0].manifest.id.clone());
		extensions.set_entries(entries);
		return;
	}
	let catalog = extensions::parse_catalog(include_bytes!("../../../extensions/catalog.json"))
		.expect("valid fixture catalog");
	extensions.set_entries(
		catalog
			.entries
			.into_iter()
			.map(|entry| ui::ExtensionEntry {
				cover_image: None,
				local_theme: false,
				manifest: entry.manifest,
				description: entry.description,
				preview: entry.preview,
				theme_preview: None,
				reviewed: true,
				sha256: entry.sha256,
				download_bytes: entry.download_bytes,
				enabled: false,
				cleanup_pending: false,
				update_available: false,
				update_manifest: None,
			})
			.collect(),
	);
	let previews = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../extensions/previews");
	{
		let (id, filename) = ("serein-ocean", "ocean.png");
		let image = image::open(previews.join(filename))
			.expect("valid fixture preview")
			.to_rgba8();
		let size = [image.width() as usize, image.height() as usize];
		extensions.preview_fixture_image(
			id.into(),
			egui::ColorImage::from_rgba_unmultiplied(size, image.as_raw()),
		);
	}
}

/// Synthetic settings for a fixture channel, including a forum's tags and post defaults.
fn channel_settings(
	state: &client_core::State,
	channel: model::Id,
) -> client_core::channel_actions::Edit {
	let source = state.channel(channel).expect("fixture channel");
	let tags = source.tags.as_deref().cloned().unwrap_or_default();
	client_core::channel_actions::Edit {
		name: source.name.clone(),
		topic: "Share one idea per post. Search first, and tag what the idea is about.".into(),
		forum: matches!(source.kind, 15 | 16).then(|| {
			Box::new(client_core::channel_actions::ForumEdit {
				tags: tags.available,
				require_tag: tags.required,
				reaction: tags.reaction,
				layout: tags.layout,
				sort: tags.sort,
				match_all: tags.match_all,
				hide_after: 4320,
				..Default::default()
			})
		}),
		..Default::default()
	}
}

/// Bounded, offline conversation for typography and spacing comparisons.
fn prime_media_embeds(state: &mut client_core::State, provider_only: bool) {
	let channel = state.selected.expect("synthetic embed conversation");
	let poster = model::EmbedMedia {
		url: Some("https://example.org/synthetic/poster.png".into()),
		proxy_url: Some("https://images-ext-1.discordapp.net/external/synthetic-poster/https/example.org/poster.png".into()),
		width: 960,
		height: 540,
		..Default::default()
	};
	let messages = (if provider_only { 2..3 } else { 0..2 })
		.rev()
		.map(|variant| {
			let mut message = test_support::message(600 + variant, channel);
			message.attachments.clear();
			message.reactions = Some(vec![]);
			message.reply_to = None;
			let embed = match variant {
				0 => model::Embed {
					kind: "rich".into(),
					url: Some("https://x.com/synthetic/status/600".into()),
					description: Some("A synthetic post with a full-size photo. The photo should sit below the text and keep its original proportions.".into()),
					author: Some(model::EmbedAuthor {
						name: "Synthetic (@synthetic)".into(),
						url: Some("https://x.com/synthetic".into()),
						..Default::default()
					}),
					thumbnail: Some(poster.clone()),
					footer: Some(model::EmbedFooter { text: "X".into(), ..Default::default() }),
					timestamp: Some("2026-10-12T17:37:03Z".into()),
					color: Some(0x1da1f2),
					..Default::default()
				},
				1 => model::Embed {
					kind: "video".into(),
					url: Some("https://example.org/synthetic/video".into()),
					title: Some("Video without size metadata".into()),
					video: Some(model::EmbedMedia {
						url: Some("https://example.org/synthetic/video.mp4".into()),
						proxy_url: Some("https://media.discordapp.net/external/synthetic-video/https/example.org/video.mp4".into()),
						..Default::default()
					}),
					..Default::default()
				},
				_ => model::Embed {
					kind: "video".into(),
					url: Some("https://www.youtube.com/watch?v=synthet1c01".into()),
					title: Some("Provider-hosted player".into()),
					provider: Some(model::EmbedAuthor { name: "YouTube".into(), ..Default::default() }),
					thumbnail: Some(poster.clone()),
					video: Some(model::EmbedMedia {
						url: Some("https://www.youtube.com/embed/synthet1c01".into()),
						width: 1280,
						height: 720,
						..Default::default()
					}),
					..Default::default()
				},
			};
			message.content = embed.url.clone().unwrap();
			message.embeds = vec![embed];
			message
		})
		.collect();
	state.timeline.clear();
	state
		.timeline
		.seed_cache(messages)
		.expect("synthetic media embeds");
}

fn prime_social_video(state: &mut client_core::State) {
	let channel = state.selected.expect("fixture channel");
	let messages = [
		(
			"Instagram",
			"https://www.oginstagram.com/reel/fixture",
			144,
			240,
		),
		(
			"Reddit",
			"https://v.redd.it/fixture/DASH_480.mp4?source=fallback",
			320,
			180,
		),
	]
	.into_iter()
	.enumerate()
	.map(|(index, (provider, url, width, height))| {
		let mut message = test_support::message(600 + index as u64, channel);
		message.content = url.into();
		message.attachments.clear();
		message.reactions = Some(vec![]);
		message.reply_to = None;
		message.embeds = vec![model::Embed {
			kind: "link".into(),
			url: Some(url.into()),
			provider: Some(model::EmbedAuthor {
				name: provider.into(),
				..Default::default()
			}),
			thumbnail: Some(model::EmbedMedia {
				proxy_url: Some(format!(
					"https://media.discordapp.net/external/fixture/https/example.org/{index}.png"
				)),
				width,
				height,
				..Default::default()
			}),
			..Default::default()
		}];
		message
	})
	.collect();
	state.timeline.clear();
	state.timeline.seed_cache(messages).expect("fixture videos");
}

fn prime_conversation(state: &mut client_core::State, inline_media: bool) {
	let channel = state.selected.expect("synthetic conversation");
	let ready = client_core::Event::Ready {
		user: state.user.clone().expect("synthetic account"),
		guilds: state.guilds.clone(),
		channels: state.channels.clone(),
		permissions: test_support::permission_snapshot(state),
	};
	*state = client_core::State {
		demo: true,
		..Default::default()
	};
	state.apply(client_core::Envelope {
		generation: state.generation,
		event: ready,
	});
	let _ = state.select(channel);
	state.older_exhausted = true;
	state.status = "Offline fixture · no network access";

	for category in state
		.channels
		.iter_mut()
		.filter(|channel| channel.kind == 4)
	{
		category.name = if category.id == model::Id(23) {
			"Welcome"
		} else {
			"Discussion"
		}
		.into();
	}
	let samples = [
		("Hey, how is the new build looking?", false, false),
		(
			"The text is easier to read. Names and timestamps stay on one line.",
			false,
			false,
		),
		("A short reply, with a corrected word.", true, false),
		(
			"This longer message wraps naturally when the window gets smaller, without adding another row just for its edited label.",
			true,
			false,
		),
		(
			"One message can have several lines.\nEach line keeps the same rhythm.\nThe next reply starts a new group.",
			false,
			false,
		),
		("That makes sense, thanks!", false, true),
		("And this is a follow-up from the same author.", false, true),
		(
			"**Bold**, *italic*, `inline code` and a [link](https://example.invalid/) still fit together.",
			true,
			false,
		),
		("Replying to the earlier update.", false, false),
		("A final short message.", false, true),
	];
	let mut messages: Vec<model::Message> = Vec::new();
	for (index, (text, edited, same_author)) in samples.into_iter().enumerate() {
		let mut message = test_support::message(
			1562072894668800000 + index as u64 * 251_658_240_000,
			channel,
		);
		message.author = test_support::message(index as u64, channel).author;
		if same_author && let Some(previous) = messages.last() {
			message.author = previous.author.clone();
		}
		message.author.primary_guild = None;
		message.author.cosmetics = None;
		message.author.name = if message.author.id == model::Id(1) {
			"Avery (synthetic)"
		} else {
			"Robin (synthetic)"
		}
		.into();
		message.content = if inline_media {
			match index {
				0 => "no <:serein_wave:9001> · inline 🙂",
				1 => "<@1> <@2> — names follow the server nickname.",
				4 => "😔",
				_ => text,
			}
		} else {
			text
		}
		.into();
		message.edited = edited;
		message.embeds.clear();
		message.attachments.clear();
		message.reactions = Some(vec![]);
		if index == 8 {
			message.reply_to = Some(messages[if inline_media { 0 } else { 1 }].id);
		}
		messages.push(message);
	}
	state.timeline.clear();
	state.read_state.reset();
	state.history_pending = false;
	state.freshness = model::Freshness::Fresh;
	let last = messages.last().unwrap().id;
	let slots = messages[..2]
		.iter()
		.map(|message| {
			Some(model::MemberSlot::Person(model::Member {
				user: message.author.clone(),
				nick: None,
				roles: vec![],
				status: Some("online".into()),
				custom_status: Some("Offline typography fixture".into()),
				activities: vec![],
				clients: Default::default(),
			}))
		})
		.collect();
	state.members = Some(model::MemberList {
		missing_identity: vec![],
		channel,
		guild: Some(model::Id(10)),
		request: 0,
		total: 2,
		start: 0,
		lazy: false,
		groups: vec![],
		ranges: vec![],
		freshness: model::Freshness::Fresh,
		slots,
	});
	if inline_media {
		if let Some(model::MemberSlot::Person(member)) = state
			.members
			.as_mut()
			.and_then(|list| list.slots[1].as_mut())
		{
			member.nick = Some("Rowan (server nickname)".into());
		}
		state
			.drafts
			.insert(channel, "<@2> no <:serein_wave:9001> 🙂".into());
	}

	state
		.timeline
		.seed_cache(messages)
		.expect("valid synthetic conversation");
	state.apply(client_core::Envelope {
		generation: state.generation,
		event: client_core::Event::ReadState(client_core::read_state::Event::Snapshot {
			partial: false,
			entries: Some(vec![(channel, Some(last), 0)]),
			version: Some(2),
		}),
	});
}

include!("fixtures/conversation_qol.rs");
include!("fixtures/native_metrics.rs");

fn apply_admin_preview(state: &mut client_core::State, command: Option<client_core::Command>) {
	if let Some(client_core::Command::ServerAdmin {
		guild,
		request,
		action,
	}) = command
	{
		let event = server_settings_demo::execute_admin(state, guild, request, *action);
		state.apply(client_core::Envelope {
			generation: state.generation,
			event,
		});
	}
}

fn prime_mod_view(state: &mut client_core::State, messaging: &mut ui::MessagingUi) {
	// A temporary editor establishes the normal synthetic permissions/member snapshot while
	// the displayed UI remains the conversation behind the real Mod View drawer.
	let mut fixture_settings = ui::MessagingUi::default();
	server_settings_demo::open(state, &mut fixture_settings);
	let guild = state.guilds[0].id;
	let command = fixture_settings.preview_server_admin(state, guild, "members");
	apply_admin_preview(state, command);
	let owner = state.user.as_ref().expect("fixture owner").id;
	let user = state
		.server_admin
		.members
		.as_ref()
		.and_then(|page| page.items.iter().find(|row| row.user.id != owner))
		.map_or(model::Id(2), |row| row.user.id);
	let command = messaging.preview_mod_view(state, guild, user);
	apply_admin_preview(state, command);
}

fn prime_mfa(state: &mut client_core::State) -> client_core::mfa::Target {
	// This is a synthetic authenticated state with no adapters. Mint the real protected
	// operation through State so the dialog exercises the production permission/scope checks.
	state.auth = client_core::auth::AuthState::Authenticated;
	state.gateway_connected = true;
	let command = state
		.save_server_settings(model::server_settings::Edit {
			name: Some("Synthetic server verification".into()),
			..Default::default()
		})
		.expect("eligible synthetic protected settings action");
	let target = client_core::mfa::Target::from_command(&command)
		.expect("production protected action scope");
	state.apply(client_core::Envelope {
		generation: state.generation,
		event: client_core::Event::Mfa(client_core::mfa::Event::Required(
			client_core::mfa::Notice {
				id: 1,
				target: target.clone(),
				methods: vec![
					client_core::mfa::Method::Totp,
					client_core::mfa::Method::Backup,
					client_core::mfa::Method::Sms,
					client_core::mfa::Method::Password,
				],
			},
		)),
	});
	target
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
	let args: Vec<_> = std::env::args().skip(1).collect();
	let value = |prefix: &str| args.iter().find_map(|arg| arg.strip_prefix(prefix));
	if !args.iter().any(|arg| arg == "--demo") {
		return Err("Usage: profile_preview --demo [--output=PATH.png | --smoke | --interactive] [--page=events|event-detail|event-edit|stage|stickers|slash-commands|slash-command-search|slash-command-options|profile|conversation|markdown|embeds|channel-links|channel-link-replies|profile-card|collectibles|member-tags|dm-tags|account|appearance|storage|voice|general|chat|language|keybinds|extensions|server|server-engagement|server-safety|server-emoji|server-stickers|server-members|server-roles|server-invites|server-integrations|server-audit-log|server-soundboard|server-automod|server-moderation|server-applications|server-access|server-bans|server-mfa|mod-view|force-sync|channels|screen-share|emoji|gif-favorites] [--screen-audio-controls] [--video-backend=ffmpeg] [--video-codec=h264|h265|av1] [--camera-resolution=480p|720p|1080p|1440p|4k|8k] [--camera-fps=15|30|60] [--command=help|weather] [--themes] [--extension=ID] [--thumbnail] [--voice-debug | --voice-connection] [--voice-channel-status] [--lavasr] [--width=1120] [--height=760] [--scroll=PIXELS] [--light] [--compact] [--inline-media] [--conversation-qol] [--reaction-history] [--metrics=PATH.json] [--timeline-churn] [--media-embeds] [--provider-embed] [--transparency=0..100]".into());
	}
	let smoke = args.iter().any(|arg| arg == "--smoke");
	let interactive = args.iter().any(|arg| arg == "--interactive");
	let output = PathBuf::from(
		value("--output=")
			.or((smoke || interactive).then_some(""))
			.ok_or("Missing --output=PATH.png")?,
	);
	let page = value("--page=").unwrap_or("profile").to_owned();
	if !matches!(
		page.as_str(),
		"profile"
			| "events"
			| "event-detail"
			| "event-edit"
			| "stage" | "stickers"
			| "slash-commands"
			| "slash-command-search"
			| "slash-command-options"
			| "profile-card"
			| "collectibles"
			| "member-tags"
			| "conversation"
			| "channels"
			| "markdown"
			| "embeds"
			| "channel-links"
			| "channel-link-replies"
			| "dm-tags"
			| "dm-intro"
			| "channel-intro"
			| "post-intro"
			| "account"
			| "appearance"
			| "voice" | "storage"
			| "general"
			| "chat" | "language"
			| "keybinds"
			| "extensions"
			| "server"
			| "server-engagement"
			| "server-stickers"
			| "server-safety"
			| "server-emoji"
			| "server-members"
			| "server-roles"
			| "server-invites"
			| "server-integrations"
			| "server-audit-log"
			| "server-soundboard"
			| "server-automod"
			| "server-moderation"
			| "server-applications"
			| "server-access"
			| "server-bans"
			| "server-mfa"
			| "mod-view"
			| "force-sync"
			| "forum" | "forum-post"
			| "forum-gallery"
			| "forum-settings"
			| "friends"
			| "screen-share"
			| "emoji" | "gif-favorites"
	) {
		return Err("Page must be events, event-detail, event-edit, stage, profile, conversation, markdown, embeds, channel-links, channel-link-replies, profile-card, collectibles, member-tags, dm-tags, account, appearance, storage, voice, general, chat, language, keybinds, extensions, slash-commands, slash-command-search, slash-command-options, server, server-engagement, server-safety, server-emoji, server-stickers, server-members, server-roles, server-invites, server-integrations, server-audit-log, server-soundboard, server-automod, server-moderation, server-applications, server-access, server-bans, server-mfa, mod-view, force-sync, screen-share, emoji or gif-favorites".into());
	}
	let slash_command = value("--command=").unwrap_or("help").to_owned();
	if !matches!(slash_command.as_str(), "help" | "weather") {
		return Err("Command fixture must be help or weather".into());
	}
	let width: f32 = value("--width=").unwrap_or("1120").parse()?;
	let height: f32 = value("--height=").unwrap_or("760").parse()?;
	let scroll = value("--scroll=")
		.map(str::parse::<f32>)
		.transpose()?
		.map(|distance| (distance, egui::pos2(width * 0.6, height * 0.5)));
	if !(500.0..=1920.0).contains(&width) || !(520.0..=1200.0).contains(&height) {
		return Err("Viewport must be 500-1920 by 520-1200".into());
	}
	let forum_tags: Vec<_> = value("--tags=")
		.map(|list| {
			list.split(',')
				.filter_map(|id| id.parse().ok())
				.map(model::Id)
				.collect()
		})
		.unwrap_or_default();
	let light = args.iter().any(|arg| arg == "--light");
	let transparency = value("--transparency=").map(str::parse::<u8>).transpose()?;
	if transparency.is_some_and(|amount| amount > 100) {
		return Err("Transparency must be in 0..=100".into());
	}
	let activities = args.iter().any(|arg| arg == "--activities");
	let friends_tab = value("--tab=").unwrap_or("online").to_owned();
	let theme_editor = value("--theme-editor=").map(str::to_owned);
	let theme_preview = args.iter().any(|arg| arg == "--theme-preview");
	let thumbnail = args.iter().any(|arg| arg == "--thumbnail");
	let extension = value("--extension=").map(str::to_owned);
	let name_style = value("--name-style=").and_then(|id| id.parse::<u32>().ok());
	let name_font = value("--name-font=").and_then(|id| id.parse::<u32>().ok());
	// `--name-style=` and `--name-font=` give the previewed user a synthetic Nitro name style;
	// a font alone uses the solid effect.
	let styled = name_style.is_some() || name_font.is_some();
	let style_name = |user: &mut model::User| {
		if styled {
			user.cosmetics.get_or_insert_default().name_style = Some(model::NameStyle {
				font: name_font.unwrap_or(0),
				effect: model::NameEffect::from_id(name_style.unwrap_or(1)),
				colors: vec![0xff7ab8, 0x7a5cff, 0x34d6ff, 0xffd166],
			});
		}
	};
	let member_presence = value("--presence=").unwrap_or("online").to_owned();
	let card_status = value("--status=").map(str::to_owned);
	let private_profile = args.iter().any(|arg| arg == "--private");
	let fixture = extension.as_deref().map(extension_fixture).transpose()?;
	let saved = Arc::new(AtomicBool::new(false));
	let completed = saved.clone();
	eframe::run_native(
		"Serein · offline profile preview",
		eframe::NativeOptions {
			viewport: egui::ViewportBuilder::default()
				.with_inner_size([width, height])
				.with_decorations(false),
			renderer: eframe::Renderer::Wgpu,
			persist_window: false,
			..Default::default()
		},
		Box::new(move |cc| {
			ui::fonts::install(&cc.egui_ctx);
			let _ = ui::emoji::install(&cc.egui_ctx);
			ui::design::set_window_effects(transparency.is_some(), transparency.unwrap_or(0), 0);
			ui::design::apply(&cc.egui_ctx);
			if args.iter().any(|arg| arg.starts_with("--metrics="))
				&& let Some(render) = cc.wgpu_render_state.as_ref()
			{
				eprintln!("Offline preview renderer: {:?}", render.adapter.get_info());
			}
			cc.egui_ctx.set_theme(if light {
				egui::ThemePreference::Light
			} else {
				egui::ThemePreference::Dark
			});
			let mut state = if matches!(
				page.as_str(),
				"slash-commands" | "slash-command-search" | "slash-command-options"
			) {
				slash_demo::preview()
			} else if matches!(
				page.as_str(),
				"events" | "event-detail" | "event-edit" | "stage"
			) {
				test_support::community_state(page == "stage")
			} else if page == "friends" {
				test_support::friends_demo_state()
			} else if page == "screen-share" {
				test_support::call_demo_state()
			} else {
				test_support::demo_state()
			};
			if activities {
				prime_activities(&mut state);
			}
			if args.iter().any(|arg| arg == "--nitro") {
				state.premium_type = 2;
			}
			if page == "embeds" && args.iter().any(|arg| arg == "--social-video-embeds") {
				prime_social_video(&mut state);
			} else if page == "embeds" && args.iter().any(|arg| arg == "--media-embeds") {
				prime_media_embeds(&mut state, args.iter().any(|arg| arg == "--provider-embed"));
			} else if page == "embeds" {
				let channel = state.selected.expect("synthetic embed conversation");
				let messages = (0..3)
					.rev()
					.map(|variant| {
						let mut message = test_support::message(600 + variant, channel);
						let url = format!("https://example.org/synthetic/post/{variant}");
						message.content = if variant == 0 {
							url.clone()
						} else {
							"Synthetic post without its source URL in the message".into()
						};
						message.attachments.clear();
						message.reactions = Some(vec![]);
						message.embeds = vec![model::Embed {
							kind: "rich".into(),
							url: Some(url),
							description: Some(
								match variant {
									0 => {
										"The message already links the source. No extra source action is needed."
									}
									1 => {
										"An untitled card with a footer and a separate author link."
									}
									_ => "An untitled card with no footer or timestamp.",
								}
								.into(),
							),
							author: Some(model::EmbedAuthor {
								name: "Synthetic (@synthetic)".into(),
								url: Some("https://example.org/synthetic/author".into()),
								..Default::default()
							}),
							footer: (variant != 2).then(|| model::EmbedFooter {
								text: "X".into(),
								..Default::default()
							}),
							timestamp: (variant != 2).then(|| "2026-10-12T17:37:03Z".into()),
							..Default::default()
						}];
						message
					})
					.collect();
				state.timeline.clear();
				state
					.timeline
					.seed_cache(messages)
					.expect("valid synthetic embeds");
			}
			if page == "markdown" {
				let channel = state.selected.expect("synthetic markdown conversation");
				let mut message = test_support::message(600, channel);
				message.content =
					include_str!("../../../crates/ui/tests/fixtures/prune-announcement.txt").into();
				message.attachments.clear();
				message.embeds.clear();
				message.reactions = Some(vec![]);
				state.timeline.clear();
				state
					.timeline
					.seed_cache(vec![message])
					.expect("valid synthetic markdown");
			}
			if page == "channel-links" {
				prime_channel_links(&mut state);
			}
			if page == "channel-link-replies" {
				prime_channel_link_replies(&mut state);
			}
			if page == "profile" {
				if let Some(own) = state.user.as_mut() {
					style_name(own);
				}
				prime_profile(&mut state);
			}
			if page == "member-tags" {
				let mut user = test_support::message(1, model::Id(20)).author;
				style_name(&mut user);
				state.members = Some(model::MemberList {
					missing_identity: vec![],
					channel: model::Id(20),
					guild: Some(model::Id(10)),
					request: 0,
					total: 1,
					lazy: false,
					groups: vec![],
					ranges: vec![],
					freshness: model::Freshness::Fresh,
					start: 0,
					slots: vec![Some(model::MemberSlot::Person(model::Member {
						user,
						nick: None,
						roles: vec![],
						// `--presence=` shows another status glyph (idle, dnd, offline).
						status: Some(member_presence.clone()),
						custom_status: Some("Building a quieter place".into()),
						activities: vec![],
						clients: model::ClientPlatforms {
							mobile: true,
							..Default::default()
						},
					}))],
				});
			} else if page == "dm-tags" {
				let _ = state.select(model::Id(22));
			} else if let Some(channel) = match page.as_str() {
				"dm-intro" => Some(model::Id(22)),
				"channel-intro" => Some(model::Id(21)),
				"post-intro" => Some(model::Id(27)),
				_ => None,
			} {
				// A short synthetic conversation whose whole history fits one page.
				state.gateway_connected = true;
				state.auth = client_core::auth::AuthState::Authenticated;
				if let Some(client_core::Command::History { request, .. }) = state.select(channel) {
					let messages = (0..4u64)
						.rev()
						.map(|n| {
							let mut message =
								test_support::message(1_000_000 + n * 9_000_000_000_000, channel);
							message.content = [
								"Hey!",
								"Synthetic hello",
								"How was the trip?",
								"Great, thanks",
							][n as usize]
								.into();
							message.attachments.clear();
							message.embeds.clear();
							message.reactions = Some(vec![]);
							message
						})
						.collect();
					state.apply(client_core::Envelope {
						generation: state.generation,
						event: client_core::Event::History {
							channel,
							request,
							older: false,
							messages,
						},
					});
				}
			} else if page.starts_with("forum") {
				state.gateway_connected = true;
				state.auth = client_core::auth::AuthState::Authenticated;
				let _ = state.select(model::Id(26));
			}
			if args.iter().any(|arg| arg == "--voice-channel-status") {
				state.apply_voice(client_core::voice::Event::ChannelStatus {
					guild: model::Id(10),
					channel: model::Id(25),
					status: Some("Building something together".into()),
				});
			}
			if page == "conversation" {
				prime_conversation(&mut state, args.iter().any(|arg| arg == "--inline-media"));
			}
			let mut messaging = ui::MessagingUi::default();
			let mut mfa_fixture_target = None;
			if styled {
				messaging.cosmetics.name_styles = true;
			}
			if args.iter().any(|arg| arg == "--compact") {
				messaging.apply_reading_preferences(
					&cc.egui_ctx,
					model::ReadingPreferences {
						compact_messages: true,
						..messaging.reading_preferences
					},
				);
			}
			messaging.transparency_blur = transparency.is_some();
			messaging.transparency = transparency.unwrap_or(0);
			messaging.tray_available = platform::tray::supported();
			messaging.startup_available = platform::startup::available();
			messaging.startup_enabled = args.iter().any(|arg| arg == "--startup-enabled");
			messaging.startup_minimized = args.iter().any(|arg| arg == "--startup-minimized");
			if matches!(page.as_str(), "events" | "event-detail" | "event-edit") {
				messaging.preview_community(
					&mut state,
					model::Id(10),
					page == "event-detail",
					page == "event-edit",
				);
			} else if page == "friends" {
				messaging.preview_friends_tab(&friends_tab);
			} else if page == "forum" {
				messaging.preview_forum(model::Id(26), &forum_tags, None);
			} else if page == "forum-gallery" {
				messaging.preview_forum(model::Id(26), &[], None);
				messaging.preview_forum_layout(model::forum::Layout::Gallery);
			} else if page == "screen-share" {
				messaging.preview_screen_share(&state);
				if args.iter().any(|arg| arg == "--screen-audio-controls") {
					messaging.screen.supported = true;
				}
			} else if page == "forum-settings" {
				messaging.preview_channel_settings(model::Id(26), state.generation);
			} else if page == "forum-post" {
				messaging.preview_forum(
					model::Id(26),
					&[model::Id(2603)],
					Some("Faster startup on older phones"),
				);
			} else if matches!(
				page.as_str(),
				"stage"
					| "member-tags" | "dm-tags"
					| "dm-intro" | "channel-intro"
					| "post-intro" | "markdown"
					| "conversation"
					| "channel-links"
					| "channel-link-replies"
					| "embeds"
			) {
				// State is primed above; the normal offline messaging surface renders the list.
			} else if page == "slash-commands" {
				messaging.preview_slash_commands();
			} else if page == "slash-command-search" {
				// A partial name: the flat "commands matching" list with per-row icons.
				let channel = state.selected.expect("synthetic command conversation");
				state
					.drafts
					.insert(channel, format!("/{}", &slash_command[..2]));
				messaging.preview_slash_commands();
			} else if page == "slash-command-options" {
				let channel = state.selected.expect("synthetic command conversation");
				state.drafts.insert(channel, format!("/{slash_command}"));
				messaging.preview_slash_command_options(&mut state);
			} else if page == "emoji" {
				messaging.preview_emoji_picker();
			} else if page == "gif-favorites" {
				let gifs = test_support::gif_page(None).gifs;
				state.restore_gif_favorites(gifs.iter().rev().cloned().collect());
				if let Some(client_core::Command::GifFavorites { request, .. }) =
					state.request_gif_favorites()
				{
					state.apply_gif_favorites(request, Ok(gifs));
				}
				messaging.preview_gif_picker("favorites");
			} else if page == "stickers" {
				test_support::seed_stickers(&mut state);
				messaging.preview_sticker_picker();
			} else if page == "collectibles" {
				// Synthetic cosmetics: the UI draws their artwork locally, offline. They are
				// opt-in, so this page turns all four on.
				messaging.cosmetics = model::CosmeticPreferences::SHOWN;
				let mut user = test_support::message(1, model::Id(20)).author;
				style_name(&mut user);
				if let Some(own) = state.user.as_mut() {
					style_name(own);
				}
				let mut offline = test_support::message(2, model::Id(20)).author;
				offline.id = model::Id(3);
				offline.name = "Sky (synthetic)".into();
				offline.cosmetics = user.cosmetics.clone();
				let member = |user: model::User, status: &str| {
					Some(model::MemberSlot::Person(model::Member {
						user,
						nick: None,
						roles: vec![],
						status: Some(status.into()),
						custom_status: None,
						activities: vec![],
						clients: model::ClientPlatforms::default(),
					}))
				};
				state.members = Some(model::MemberList {
					missing_identity: vec![],
					channel: model::Id(20),
					guild: Some(model::Id(10)),
					request: 0,
					total: 3,
					start: 0,
					lazy: false,
					groups: vec![],
					ranges: vec![],
					freshness: model::Freshness::Fresh,
					slots: vec![
						member(user.clone(), "online"),
						member(test_support::message(2, model::Id(20)).author, "online"),
						member(offline, "offline"),
					],
				});
				let own = state.user.clone().expect("synthetic account");
				if args.iter().any(|arg| arg == "--full") {
					messaging.preview_full_profile(user);
				} else if args.iter().any(|arg| arg == "--badge-directory") {
					messaging.preview_badges(
						own,
						Some(model::badges::id::ACCOUNT_AGE),
						egui::Pos2::ZERO,
					);
				} else if args.iter().any(|arg| arg == "--badge-customize") {
					messaging.preview_badges(own, None, egui::pos2(1010.0, 330.0));
				} else if args.iter().any(|arg| arg == "--own") {
					messaging.preview_profile_popout(own);
				} else {
					messaging.preview_profile_popout(user);
				}
			} else if page == "profile-card" {
				state.demo = false;
				state.gateway_connected = true;
				let mut user = test_support::message(1, model::Id(20)).author;
				style_name(&mut user);
				state.members = Some(model::MemberList {
					missing_identity: vec![],
					channel: model::Id(20),
					guild: Some(model::Id(10)),
					request: 0,
					total: 1,
					start: 0,
					lazy: false,
					groups: vec![],
					ranges: vec![],
					freshness: model::Freshness::Fresh,
					slots: vec![Some(model::MemberSlot::Person(model::Member {
						user: user.clone(),
						nick: None,
						roles: vec![],
						status: Some("online".into()),
						// `--status=` gives the synthetic member a custom status.
						custom_status: card_status.clone(),
						activities: vec![],
						clients: model::ClientPlatforms {
							mobile: true,
							..Default::default()
						},
					}))],
				});
				// `--private` serves the card a private profile, as Discord sends a non-friend.
				let guild = Some(model::Id(10));
				if private_profile
					&& let Some(client_core::Command::Profile { request, .. }) =
						state.request_profile(user.id, guild)
				{
					let mut profile = ui::synthetic_own_profile(&user);
					profile.private = true;
					profile.bio.clear();
					profile.badges.clear();
					profile.connections.clear();
					state.apply(client_core::Envelope {
						generation: state.generation,
						event: client_core::Event::Profile {
							user: user.id,
							guild,
							request,
							result: Ok(Box::new(profile)),
						},
					});
					state.clear_profile();
				}
				messaging.preview_profile(user);
			} else if let Some((package, invocation, result)) = fixture {
				prime_extension_chat(&mut state);
				if let Some(theme) = package.theme.as_ref() {
					ui::design::set_extension_theme(Some(theme));
					ui::design::apply(&cc.egui_ctx);
				}
				if let Some(output) = result {
					if matches!(package.manifest.id.as_str(), "custom-rpc" | "api-proxy") {
						messaging.extensions.set_entries(vec![ui::ExtensionEntry {
							description: package.manifest.name.clone(),
							preview: None,
							theme_preview: None,
							cover_image: None,
							local_theme: false,
							manifest: package.manifest.clone(),
							reviewed: true,
							sha256: String::new(),
							download_bytes: 0,
							enabled: true,
							cleanup_pending: false,
							update_available: false,
							update_manifest: None,
						}]);
						messaging.extensions.present_output(
							package.manifest.id.clone(),
							invocation,
							ui::ExtensionContext::panel(&state),
							output.clone(),
							&state,
						);
					}
					if output.preserve_deleted_messages {
						let channel = state.selected.unwrap();
						state.apply(client_core::Envelope {
							generation: state.generation,
							event: client_core::Event::Delete {
								channel,
								id: model::Id(601),
							},
						});
					}
				}
			} else if page == "channels" {
				// The historical fixture omits this setting; demonstrate the loaded
				// controls while separate regressions cover unknown preferences.
				state.apply(client_core::Envelope {
					generation: state.generation,
					event: client_core::Event::NotificationPreferences(
						client_core::notifications::Event::Updates {
							entries: vec![model::guild_preferences::Update {
								guild: Some(model::Id(10)),
								fields: model::guild_preferences::Fields {
									hide_muted: model::Patch::Value(false),
									..Default::default()
								},
								..Default::default()
							}],
							replace: false,
						},
					),
				});
				messaging.preview_browse_channels(&mut state, model::Id(10));
			} else if page == "force-sync" {
				messaging.preview_force_sync();
			} else if page == "mod-view" {
				prime_mod_view(&mut state, &mut messaging);
			} else if page.starts_with("server") {
				server_settings_demo::open(&mut state, &mut messaging);
				if page == "server-mfa" {
					mfa_fixture_target = Some(prime_mfa(&mut state));
				} else if page == "server-engagement" {
					messaging.preview_server_engagement();
				} else if page == "server-safety" {
					messaging.preview_server_safety();
				} else if page != "server"
					&& let Some(client_core::Command::ServerAdmin {
						guild,
						request,
						action,
					}) = messaging.preview_server_admin(
						&mut state,
						model::Id(10),
						page.trim_start_matches("server-"),
					) {
					let event =
						server_settings_demo::execute_admin(&state, guild, request, *action);
					state.apply(client_core::Envelope {
						generation: state.generation,
						event,
					});
				}
			} else {
				if page == "voice" {
					if args.iter().any(|arg| arg == "--lavasr") {
						messaging.voice_processing.edit().suppression =
							model::voice_settings::NoiseSuppression::LavaSrV2;
					}
					use model::voice_settings::{VideoCodec, VideoFrameRate, VideoResolution};
					if let Some(backend) = args
						.iter()
						.find_map(|arg| arg.strip_prefix("--video-backend="))
						&& backend != "ffmpeg"
					{
						return Err("Video backend must be ffmpeg".into());
					}
					if let Some(codec) = args
						.iter()
						.find_map(|arg| arg.strip_prefix("--video-codec="))
					{
						messaging.video_settings.codec = match codec {
							"h264" => VideoCodec::H264,
							"h265" => VideoCodec::H265,
							"av1" => VideoCodec::Av1,
							_ => return Err("Video codec must be h264, h265 or av1".into()),
						};
					}
					if let Some(resolution) = args
						.iter()
						.find_map(|arg| arg.strip_prefix("--camera-resolution="))
					{
						messaging.video_settings.camera_resolution =
							match resolution {
								"480p" => VideoResolution::P480,
								"720p" => VideoResolution::P720,
								"1080p" => VideoResolution::P1080,
								"1440p" => VideoResolution::P1440,
								"4k" => VideoResolution::P2160,
								"8k" => VideoResolution::P4320,
								_ => return Err(
									"Camera resolution must be 480p, 720p, 1080p, 1440p, 4k or 8k"
										.into(),
								),
							};
					}
					if let Some(fps) = args
						.iter()
						.find_map(|arg| arg.strip_prefix("--camera-fps="))
					{
						messaging.video_settings.camera_frame_rate = match fps {
							"15" => VideoFrameRate::Fps15,
							"30" => VideoFrameRate::Fps30,
							"60" => VideoFrameRate::Fps60,
							_ => return Err("Camera frame rate must be 15, 30 or 60".into()),
						};
					}
				}
				messaging.preview_settings(
					if page == "extensions" && args.iter().any(|arg| arg == "--themes") {
						"themes"
					} else {
						&page
					},
				);
				if page == "storage" {
					// Synthetic measurements; the demo image worker keeps no disk cache.
					let media = 318 * 1024 * 1024;
					let artwork = 9 * 1024 * 1024 + 300 * 1024;
					let history = 120 * 1024 * 1024;
					let metadata = 4 * 1024 * 1024;
					messaging.force_sync_cache = Some(ui::force_sync::CacheSummary {
						total_bytes: media + artwork + history + metadata,
						history_bytes: Some(history),
						metadata_bytes: Some(metadata),
						media_bytes: Some(media),
						artwork_bytes: Some(artwork),
						last_cleanup: Some("force-sync-cache-cleaned"),
					});
					messaging.image_cache_usage = Some([
						model::CacheUsage {
							bytes: 318 * 1024 * 1024,
							files: 1_204,
						},
						model::CacheUsage {
							bytes: 9 * 1024 * 1024 + 300 * 1024,
							files: 642,
						},
					]);
				}
				if page == "chat" {
					// The app applies saved preferences at startup; this fixture skips them.
					messaging.preload_on_hover =
						local_store::AppPreferences::default().preload_on_hover;
				}
				if page == "extensions" {
					seed_catalog(
						&mut messaging.extensions,
						args.iter().any(|arg| arg == "--themes"),
					);
					messaging
						.extensions
						.preview_themes(args.iter().any(|arg| arg == "--themes"));
					if theme_preview {
						prime_extension_chat(&mut state);
						let package = extensions::parse_package(include_bytes!(
							"../../../extensions/themes/katana.serein-extension"
						))?;
						messaging.extensions.receive_theme_edit(
							Box::new(package),
							None,
							None,
							false,
							true,
						);
					}
					if let Some(tab) = &theme_editor {
						let mut package = extensions::parse_package(include_bytes!(
							"../../../extensions/themes/ocean.serein-extension"
						))
						.expect("valid theme fixture");
						package.manifest.name = "My ocean".into();
						package.manifest.author = "You".into();
						messaging.extensions.receive_theme_edit(
							Box::new(package),
							None,
							None,
							true,
							false,
						);
						let bytes = include_bytes!("../../../extensions/previews/ocean.png");
						let pixels = image::load_from_memory(bytes)
							.expect("valid fixture image")
							.to_rgba8();
						messaging.extensions.receive_theme_image(
							bytes.to_vec(),
							Arc::new(egui::ColorImage::from_rgba_unmultiplied(
								[pixels.width() as usize, pixels.height() as usize],
								pixels.as_raw(),
							)),
						);
						messaging.extensions.preview_theme_editor_tab(tab);
					}
				}
			}
			if args
				.iter()
				.any(|arg| matches!(arg.as_str(), "--voice-debug" | "--voice-connection"))
			{
				messaging.preview_voice_connection(
					&state,
					args.iter().any(|arg| arg == "--voice-debug"),
				);
			}
			if args.iter().any(|arg| arg == "--conversation-qol") {
				prime_conversation_qol(&mut state, &mut messaging);
			}
			if args.iter().any(|arg| arg == "--reaction-history") {
				// Synthetic account settings only; never contacts Discord.
				state.reaction_preferences.ranked = discord_protocol::reaction_preferences(br#"{"settings":"ajIKCgoEOTAwMRICIGQKDAoGcm9ja2V0EgIgWgoKCgRleWVzEgIgUAoKCgRmaXJlEgIgRg=="}"#).expect("synthetic reaction settings");
				state.reaction_preferences.revision += 1;
			}
			if args.iter().any(|arg| arg == "--timeline-churn") {
				prime_timeline_churn(&mut state);
			}
			Ok(Box::new(Preview {
				inbox_demo: inbox_demo::Store::default(),
				mfa_fixture_target,
				timeline_churn: args.iter().any(|arg| arg == "--timeline-churn"),
				interactive,
				metrics: args
					.iter()
					.find_map(|arg| arg.strip_prefix("--metrics="))
					.map(NativeMetrics::new),
				smoke,
				messaging,
				state,
				output,
				thumbnail,
				frames: 0,
				scroll,
				requested: false,
				screenshot: None,
				writer: None,
				saved: completed,
				started: Instant::now(),
			}))
		}),
	)?;
	if !interactive && !saved.load(Ordering::Acquire) {
		return Err("No screenshot saved".into());
	}
	Ok(())
}

#[cfg(test)]
mod tests {
	#[test]
	fn mod_view_fixture_uses_scoped_production_review_without_replacing_chat() {
		let mut state = test_support::demo_state();
		let selected = state.selected;
		let mut messaging = ui::MessagingUi::default();
		super::prime_mod_view(&mut state, &mut messaging);
		assert_eq!(state.selected, selected);
		assert!(!state.server_admin.pending);
		let review = state.server_admin.member_review.as_ref().unwrap();
		assert_eq!(Some(review.guild), state.server_admin.guild);
		assert_ne!(review.member.user.id, state.user.as_ref().unwrap().id);
	}

	#[test]
	fn verification_fixture_mints_the_real_protected_action_without_any_proof() {
		let mut state = test_support::demo_state();
		let mut messaging = ui::MessagingUi::default();
		super::server_settings_demo::open(&mut state, &mut messaging);
		let target = super::prime_mfa(&mut state);
		assert_eq!(Some(target.guild()), state.server_settings.guild);
		assert!(state.server_settings.pending);
		assert_eq!(
			state.mfa.pending.as_ref().unwrap().phase,
			client_core::mfa::Phase::Editing
		);
		let command = state
			.submit_mfa(client_core::mfa::Method::Totp, "123456".into())
			.unwrap();
		let client_core::Command::Mfa(request) = command else {
			panic!("production verification command");
		};
		assert!(state.mfa_command_allowed(&request));
	}

	#[test]
	fn non_theme_catalog_loads_community_preview() {
		super::seed_catalog(&mut ui::ExtensionUi::default(), false);
	}
}
