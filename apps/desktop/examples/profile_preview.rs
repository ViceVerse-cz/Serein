//! Offline native framebuffer capture; no account, filesystem cache, or network adapters.
use eframe::egui;
#[path = "../src/server_settings_demo.rs"]
mod server_settings_demo;
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
	messaging: ui::MessagingUi,
	state: client_core::State,
	output: PathBuf,
	thumbnail: bool,
	frames: u8,
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

	fn ui(&mut self, ui: &mut egui::Ui, _: &mut eframe::Frame) {
		let ctx = ui.ctx().clone();
		ui::design::paint_backdrop(&ctx);
		// Only synthetic fixtures execute these commands; no service adapters exist here.
		for command in self.messaging.show(ui, &mut self.state) {
			let event = match command {
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
		let profile = ui::synthetic_own_profile(state.user.as_ref().unwrap());
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
	let bytes: &[u8] = match id {
		"serein-ocean" => include_bytes!("../../../extensions/ocean.serein-extension"),
		"message-delete-protector" => include_bytes!(
			"../../../examples/extensions/packages/message-delete-protector.serein-extension"
		),
		"serein-midnight" => include_bytes!("../../../extensions/midnight.serein-extension"),
		"serein-rose" => include_bytes!("../../../extensions/rose.serein-extension"),
		"serein-forest" => include_bytes!("../../../extensions/forest.serein-extension"),
		"serein-latte" => include_bytes!("../../../extensions/latte.serein-extension"),
		_ => return Err("Unknown fixture extension".into()),
	};
	let package = extensions::parse_package(bytes)?;
	let invocation = extensions::Invocation {
		action: "activate".into(),
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
				include_bytes!("../../../extensions/ocean.serein-extension"),
				"",
			),
			(
				include_bytes!("../../../extensions/obsidian.serein-extension"),
				"Obsidian violet surfaces and lavender accents.",
			),
			(
				include_bytes!("../../../extensions/forest.serein-extension"),
				"Calm forest greens and fresh leafy accents.",
			),
			(
				include_bytes!("../../../extensions/latte.serein-extension"),
				"Warm coffee tones and a creamy caramel accent.",
			),
			(
				include_bytes!("../../../extensions/rose.serein-extension"),
				"Soft rose accents.",
			),
			(
				include_bytes!("../../../extensions/midnight.serein-extension"),
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

fn main() -> Result<(), Box<dyn std::error::Error>> {
	let args: Vec<_> = std::env::args().skip(1).collect();
	let value = |prefix: &str| args.iter().find_map(|arg| arg.strip_prefix(prefix));
	if !args.iter().any(|arg| arg == "--demo") {
		return Err("Usage: profile_preview --demo --output=PATH.png [--page=profile|profile-card|member-tags|dm-tags|account|appearance|general|extensions] [--themes] [--extension=ID] [--thumbnail] [--width=1120] [--height=760] [--light]".into());
	}
	let output = PathBuf::from(value("--output=").ok_or("Missing --output=PATH.png")?);
	let page = value("--page=").unwrap_or("profile").to_owned();
	if !matches!(
		page.as_str(),
		"profile"
			| "profile-card"
			| "member-tags"
			| "dm-tags"
			| "account"
			| "appearance"
			| "general"
			| "extensions"
			| "server"
			| "server-engagement"
	) {
		return Err("Page must be profile, profile-card, member-tags, dm-tags, account, appearance, general, extensions, server or server-engagement".into());
	}
	let width: f32 = value("--width=").unwrap_or("1120").parse()?;
	let height: f32 = value("--height=").unwrap_or("760").parse()?;
	if !(500.0..=1920.0).contains(&width) || !(520.0..=1200.0).contains(&height) {
		return Err("Viewport must be 500-1920 by 520-1200".into());
	}
	let light = args.iter().any(|arg| arg == "--light");
	let theme_editor = value("--theme-editor=").map(str::to_owned);
	let theme_preview = args.iter().any(|arg| arg == "--theme-preview");
	let thumbnail = args.iter().any(|arg| arg == "--thumbnail");
	let extension = value("--extension=").map(str::to_owned);
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
			ui::design::apply(&cc.egui_ctx);
			cc.egui_ctx.set_theme(if light {
				egui::ThemePreference::Light
			} else {
				egui::ThemePreference::Dark
			});
			let mut state = test_support::demo_state();
			if page == "profile" {
				prime_profile(&mut state);
			}
			if page == "member-tags" {
				let user = test_support::message(1, model::Id(20)).author;
				state.members = Some(model::MemberList {
					channel: model::Id(20),
					guild: Some(model::Id(10)),
					request: 0,
					total: 1,
					freshness: model::Freshness::Fresh,
					rows: vec![Some(model::Member {
						user,
						nick: None,
						roles: vec![],
						status: Some("online".into()),
						custom_status: Some("Building a quieter place".into()),
						activities: vec![],
						clients: model::ClientPlatforms {
							mobile: true,
							..Default::default()
						},
					})],
				});
			} else if page == "dm-tags" {
				let _ = state.select(model::Id(22));
			}
			let mut messaging = ui::MessagingUi::default();
			messaging.tray_available = platform::tray::supported();
			messaging.startup_available = platform::startup::available();
			messaging.startup_enabled = args.iter().any(|arg| arg == "--startup-enabled");
			messaging.startup_minimized = args.iter().any(|arg| arg == "--startup-minimized");
			if matches!(page.as_str(), "member-tags" | "dm-tags") {
				// State is primed above; the normal offline messaging surface renders the list.
			} else if page == "profile-card" {
				state.demo = false;
				state.gateway_connected = true;
				let user = test_support::message(1, model::Id(20)).author;
				state.members = Some(model::MemberList {
					channel: model::Id(20),
					guild: Some(model::Id(10)),
					request: 0,
					total: 1,
					freshness: model::Freshness::Fresh,
					rows: vec![Some(model::Member {
						user: user.clone(),
						nick: None,
						roles: vec![],
						status: Some("online".into()),
						custom_status: None,
						activities: vec![],
						clients: model::ClientPlatforms {
							mobile: true,
							..Default::default()
						},
					})],
				});
				messaging.preview_profile(user);
			} else if let Some((package, _invocation, result)) = fixture {
				prime_extension_chat(&mut state);
				if let Some(theme) = package.theme.as_ref() {
					ui::design::set_extension_theme(Some(theme));
					ui::design::apply(&cc.egui_ctx);
				}
				if let Some(output) = result {
					state.set_preserve_deleted_messages(output.preserve_deleted_messages);
					let channel = state.selected.unwrap();
					state.apply(client_core::Envelope {
						generation: state.generation,
						event: client_core::Event::Delete {
							channel,
							id: model::Id(601),
						},
					});
				}
			} else if page.starts_with("server") {
				server_settings_demo::open(&mut state, &mut messaging);
				if page == "server-engagement" {
					messaging.preview_server_engagement();
				}
			} else {
				messaging.preview_settings(
					if page == "extensions" && args.iter().any(|arg| arg == "--themes") {
						"themes"
					} else {
						&page
					},
				);
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
							"../../../extensions/katana.serein-extension"
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
							"../../../extensions/ocean.serein-extension"
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
			Ok(Box::new(Preview {
				messaging,
				state,
				output,
				thumbnail,
				frames: 0,
				requested: false,
				screenshot: None,
				writer: None,
				saved: completed,
				started: Instant::now(),
			}))
		}),
	)?;
	if !saved.load(Ordering::Acquire) {
		return Err("No screenshot saved".into());
	}
	Ok(())
}
