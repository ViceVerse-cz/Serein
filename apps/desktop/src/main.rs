#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]
#[cfg(feature = "demo")]
mod access_marks_demo;
mod app_settings;
mod audio;
mod avatars;
mod cache;
mod captcha;
#[cfg(feature = "demo")]
mod channel_demo;
mod clipboard;
#[cfg(feature = "demo")]
mod components_demo;
mod connection;
mod credentials;
#[cfg(feature = "demo")]
mod dm_demo;
mod downloads;
mod emoji_upload;
mod extension_app;
mod extension_bridge;
mod extension_data_events;
mod extension_events;
mod extension_forum_data;
mod extension_member_details;
mod extension_message_content;
mod extensions;
mod font_import;
mod game_activity;
mod gpu;
mod group_icon;
mod interaction_uploads;
mod notification_runtime;
mod notification_sounds;
mod pointer;
#[cfg(feature = "demo")]
mod post_menu_demo;
mod reading_settings;
#[cfg(feature = "demo")]
mod rendering_demo;
mod screen;
#[cfg(feature = "demo")]
mod server_settings_demo;
#[cfg(feature = "demo")]
mod slash_demo;
mod spotify;
mod startup;
mod sticker_upload;
mod toggle_setting;
mod tray_window;
mod updater;
mod uploads;
mod video;
mod voice;
mod watch;
use client_core::{
	Command, Envelope, Event, State,
	auth::{AuthState, Failure, SessionSecret},
};
use eframe::egui;
use model::Delivery;
use std::{sync::Arc, time::Duration};
use zeroize::Zeroizing;

/// Sign-in header strip: doubles as the window drag region, so it clears the traffic lights.
const SIGN_IN_HEADER_HEIGHT: f32 = if cfg!(target_os = "windows") {
	44.0
} else {
	60.0
};

fn main() -> eframe::Result {
	#[cfg(all(debug_assertions, feature = "demo"))]
	if std::env::args().any(|arg| arg == "--demo")
		&& std::env::args().any(|arg| arg == "--demo-check-call-cues")
	{
		voice::debug_call_cues_check();
		return Ok(());
	}
	#[cfg(all(debug_assertions, feature = "demo"))]
	if std::env::args().any(|arg| arg == "--demo")
		&& std::env::args().any(|arg| arg == "--demo-check-customization")
	{
		font_import::debug_check();
		let mut state = test_support::chat_demo_state();
		let mut permissions = test_support::permission_snapshot(&state);
		for guild in &mut permissions.guilds {
			guild.owner = state.user.as_ref().map(|user| user.id);
		}
		state.permissions.replace(permissions).unwrap();
		ui::debug_channel_creation(state);
		return Ok(());
	}
	#[cfg(all(debug_assertions, feature = "demo"))]
	if std::env::args().any(|arg| arg == "--demo")
		&& std::env::args().any(|arg| arg == "--demo-check-spotify")
	{
		spotify::debug_check();
		discord_api::spotify::debug_check();
		discord_gateway::debug_spotify_check();
		return Ok(());
	}
	#[cfg(all(debug_assertions, feature = "demo"))]
	if std::env::args().any(|arg| arg == "--demo")
		&& std::env::args().any(|arg| arg == "--demo-check-forward")
	{
		let mut state = test_support::demo_state();
		ui::debug_forward_check(&mut state);
		println!(
			"Forward debug check passed: picker, bounded destinations, optional note, draft preservation and queue rejection."
		);
		return Ok(());
	}
	#[cfg(all(debug_assertions, feature = "demo"))]
	if std::env::args().any(|arg| arg == "--demo")
		&& std::env::args().any(|arg| arg == "--demo-check-notification-click")
	{
		let mut state = test_support::demo_state();
		for channel in [model::Id(22), model::Id(20)] {
			let target = platform::notifications::debug_activation_check(channel);
			state.select(target);
			assert_eq!(state.selected, Some(channel));
		}
		println!(
			"Notification click debug check passed: DM/guild navigation and stale click rejection."
		);
		return Ok(());
	}
	#[cfg(all(debug_assertions, feature = "demo"))]
	if std::env::args().any(|arg| arg == "--demo")
		&& std::env::args().any(|arg| arg == "--demo-check-settings-sliders")
	{
		ui::design::debug_slider_check();
		return Ok(());
	}

	#[cfg(all(debug_assertions, feature = "demo"))]
	if std::env::args().any(|arg| arg == "--demo")
		&& std::env::args().any(|arg| arg == "--demo-check-mic-preview")
	{
		voice::debug_mic_preview_check();
		return Ok(());
	}

	#[cfg(all(debug_assertions, feature = "demo"))]
	if std::env::args().any(|arg| arg == "--demo")
		&& std::env::args().any(|arg| arg == "--demo-check-components")
	{
		components_demo::check();
		return Ok(());
	}
	#[cfg(all(debug_assertions, feature = "demo"))]
	if std::env::args().any(|arg| arg == "--demo")
		&& std::env::args().any(|arg| arg == "--demo-check-suggestion-clicks")
	{
		let mut state = test_support::demo_state();
		let channel = state
			.channels
			.iter()
			.find(|c| c.guild.is_some() && c.kind == 0)
			.unwrap()
			.id;
		state.select(channel);
		state.gateway_connected = true;
		ui::debug_suggestion_pointer_check(&mut state, channel);
		println!(
			"Suggestion pointer debug check passed: members, emoji and channels insert on click without sending."
		);
		return Ok(());
	}

	#[cfg(all(debug_assertions, feature = "demo"))]
	if std::env::args().any(|arg| arg == "--demo")
		&& std::env::args().any(|arg| arg == "--demo-check-member-search")
	{
		let mut state = test_support::demo_state();
		let channel = state
			.channels
			.iter()
			.find(|c| c.guild.is_some() && c.kind == 0)
			.unwrap()
			.id;
		state.select(channel);
		state.gateway_connected = true;
		let Some(Command::MemberSearch(request)) = state.search_members(channel, "Outside", 0)
		else {
			panic!("search should be permitted");
		};
		let event = discord_gateway::debug_member_search_check(request.clone());
		state.apply(Envelope {
			generation: state.generation,
			event,
		});
		ui::debug_member_search_check(&state, channel);
		let Some(Command::MemberSearch(_)) = state.search_members(channel, "Replacement", 0) else {
			panic!("replacement search");
		};
		state.apply(Envelope {
			generation: state.generation,
			event: Event::MemberSearch {
				request,
				result: Ok(vec![]),
			},
		});
		assert!(
			!state.member_search[0].finished,
			"stale result must be ignored"
		);
		state.demo = false;
		let user = model::Id(987654321);
		let Some(Command::MemberSearch(author_request)) = state.request_author_members(&[user])
		else {
			panic!("visible author lookup should be permitted");
		};
		assert!(state.request_author_members(&[user]).is_none());
		let guild = author_request.guild;
		let mut event = discord_gateway::debug_member_search_check(author_request);
		let role = model::Id(987654322);
		state.apply(Envelope {
			generation: state.generation,
			event: Event::Permissions(client_core::permissions::Event::Role {
				guild,
				role: model::permissions::Role {
					id: role,
					name: "Verified".into(),
					color: 0x00ff00,
					position: 1,
					hoist: false,
					bits: 0,
				},
			}),
		});
		if let Event::MemberSearch {
			result: Ok(rows), ..
		} = &mut event
		{
			rows[0].roles = vec![role];
		}
		let mut message = test_support::message(987654323, channel);
		message.author.id = user;
		message.author_roles.clear();
		state.members = None;
		assert_eq!(state.message_author_color(&message), None);
		state.apply(Envelope {
			generation: state.generation,
			event,
		});
		assert_eq!(state.message_author_color(&message), Some(0x00ff00));
		assert!(state.request_author_members(&[user]).is_none());
		assert_eq!(
			state.member_search[0].request.as_ref().unwrap().query,
			"Replacement"
		);

		println!(
			"Member search debug check passed: remote mentions and visible-author role colors use bounded, independent Gateway lookups."
		);
		return Ok(());
	}

	let demo = std::env::args().any(|arg| arg == "--demo");
	let start_minimized = startup::minimized_launch(demo, std::env::args());
	if !cfg!(feature = "demo")
		&& std::env::args().any(|arg| arg == "--demo" || arg.starts_with("--demo-"))
	{
		eprintln!("Demo support is not included; rebuild with --features demo and run with --demo");
		std::process::exit(2);
	}
	let frame_sample = std::env::args()
		.find_map(|arg| arg.strip_prefix("--demo-frame-sample").map(str::to_owned))
		.map(|value| {
			parse_frame_sample(
				demo && std::env::args().any(|arg| arg == "--demo-friends"),
				&value,
			)
			.unwrap_or_else(|reason| {
				eprintln!("{reason}");
				std::process::exit(2);
			})
		});
	#[cfg(feature = "demo")]
	if demo && std::env::args().any(|arg| arg == "--demo-check-access-marks") {
		access_marks_demo::check();
		return Ok(());
	}
	#[cfg(feature = "demo")]
	if demo && std::env::args().any(|arg| arg == "--demo-check-switcher") {
		dm_demo::check();
		return Ok(());
	}
	#[cfg(feature = "demo")]
	if demo && std::env::args().any(|arg| arg == "--demo-check-slash-commands") {
		slash_demo::check();
		return Ok(());
	}
	#[cfg(feature = "demo")]
	if demo && std::env::args().any(|arg| arg == "--demo-check-post-menu") {
		post_menu_demo::check();
		return Ok(());
	}
	#[cfg(feature = "demo")]
	if demo && std::env::args().any(|arg| arg == "--demo-check-extensions") {
		demo_check_extensions();
		return Ok(());
	}
	#[cfg(feature = "demo")]
	if demo && std::env::args().any(|arg| arg == "--demo-check-updates") {
		demo_check_updates();
		return Ok(());
	}
	// Native GPU/window capabilities are selected before the first window exists.
	let (gpu_preference, transparency_available, hide_window_decorations) = if demo {
		(model::GpuPreference::default(), false, false)
	} else {
		local_store::LocalStore::open_default()
			.and_then(|store| store.app_preferences())
			.map(|preferences| {
				(
					preferences.gpu_preference,
					preferences.transparency_blur,
					preferences.hide_window_decorations,
				)
			})
			.unwrap_or_default()
	};
	#[cfg(feature = "demo")]
	let transparency_available =
		transparency_available || demo && std::env::args().any(|arg| arg == "--demo-transparency");
	#[cfg(target_os = "windows")]
	let icon = include_bytes!("../../../packaging/windows/serein.png").as_slice();
	#[cfg(target_os = "linux")]
	let icon = include_bytes!("../../../packaging/linux/hicolor/256x256/apps/serein.png").as_slice();
	let options = eframe::NativeOptions {
		viewport: {
			let builder = egui::ViewportBuilder::default()
				.with_transparent(transparency_available)
				.with_inner_size([1120.0, 760.0])
				.with_min_inner_size([760.0, 520.0])
				.with_active(!start_minimized)
				.with_app_id("cz.viceverse.serein");
			#[cfg(any(target_os = "windows", target_os = "linux"))]
			let builder = builder
				.with_icon(eframe::icon_data::from_png_bytes(icon).expect("bundled app icon"));
			if cfg!(target_os = "macos") {
				// Discord-style inline title bar: traffic lights sit over the app's own strip.
				builder
					.with_title_shown(false)
					.with_titlebar_shown(false)
					.with_fullsize_content_view(true)
			} else if cfg!(target_os = "windows") {
				// The app paints its own caption strip and buttons; see `ui::design::window_controls`.
				builder.with_decorations(false)
			} else if cfg!(target_os = "linux") {
				builder.with_decorations(!hide_window_decorations)
			} else {
				builder
			}
		},
		renderer: eframe::Renderer::Wgpu,
		wgpu_options: eframe::egui_wgpu::WgpuConfiguration {
			wgpu_setup: eframe::egui_wgpu::WgpuSetup::CreateNew(
				eframe::egui_wgpu::WgpuSetupCreateNew {
					#[cfg(target_os = "windows")]
					instance_descriptor: {
						let mut descriptor =
							eframe::wgpu::InstanceDescriptor::new_without_display_handle_from_env();
						// HWND swapchains ignore alpha. DirectComposition keeps DX12 while
						// presenting the transparent surface selected at startup.
						if transparency_available {
							descriptor.backend_options.dx12.presentation_system =
								eframe::wgpu::Dx12SwapchainKind::DxgiFromVisual.with_env();
						}
						// Avoid Intel Vulkan driver startup crashes; keep the diagnostic override.
						descriptor.backends = eframe::wgpu::Backends::from_env()
							.unwrap_or(eframe::wgpu::Backends::DX12);
						descriptor
					},
					// Only adapters that can present to this window are eligible; the saved
					// preference just orders them. A power hint alone picks GPUs the display is
					// not wired to, which fails outright on Wayland.
					native_adapter_selector: Some(std::sync::Arc::new(
						move |adapters: &[eframe::wgpu::Adapter],
						      surface: Option<&eframe::wgpu::Surface<'_>>| {
							gpu::select(gpu_preference, adapters, surface)
						},
					)),
					..eframe::egui_wgpu::WgpuSetupCreateNew::without_display_handle()
				},
			),
			// Keep cursor-driven redraws synchronized even where AutoVsync selects FifoRelaxed.
			surface: eframe::egui_wgpu::SurfaceConfig {
				present_mode: eframe::wgpu::PresentMode::Fifo,
				..eframe::egui_wgpu::SurfaceConfig::LOW_LATENCY
			},
			..Default::default()
		},
		persist_window: false,
		persistence_path: None,
		..Default::default()
	};
	eframe::run_native(
		"Serein",
		options,
		Box::new(move |cc| {
			let desktop = Desktop::new(cc, demo, frame_sample, transparency_available)?;
			if start_minimized {
				cc.egui_ctx
					.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
			}
			Ok(Box::new(desktop))
		}),
	)
}
/// Offline updater flow and settings rendering; never opens an account or installs a package.
#[cfg(feature = "demo")]
fn demo_check_updates() {
	updater::debug_check().expect("offline updater validation");
	let ctx = egui::Context::default();
	ui::fonts::install(&ctx);
	ui::design::apply(&ctx);
	let runtime = tokio::runtime::Builder::new_current_thread()
		.enable_all()
		.build()
		.unwrap();
	let mut updater = updater::Updater::new(true);
	let mut messaging = ui::MessagingUi::default();
	messaging.build.version = env!("CARGO_PKG_VERSION");
	messaging.updates.auto_update = false;
	messaging.updates.check_requested = true;
	assert!(!updater.sync(&ctx, &runtime, &mut messaging.updates, false));
	assert!(messaging.updates.available && !messaging.updates.ready);
	messaging.updates.download_requested = true;
	assert!(!updater.sync(&ctx, &runtime, &mut messaging.updates, false));
	assert!(messaging.updates.ready);
	messaging.updates.restart_requested = true;
	assert!(!updater.sync(&ctx, &runtime, &mut messaging.updates, false));
	let restored: local_store::AppPreferences = serde_json::from_str("{}").unwrap();
	assert!(!restored.auto_update && restored.update_nightly);
	let mut settings = app_settings::Settings::default();
	messaging.updates.nightly = true;
	settings.observe(&messaging);
	let encoded = serde_json::to_string(&settings.current).unwrap();
	settings.current = serde_json::from_str(&encoded).unwrap();
	settings.apply(&mut messaging);
	assert!(!messaging.updates.auto_update && messaging.updates.nightly);
	messaging.open_update_settings();
	let mut state = test_support::demo_state();
	for size in [[1120.0, 760.0], [760.0, 520.0]] {
		for theme in [egui::ThemePreference::Dark, egui::ThemePreference::Light] {
			ctx.set_theme(theme);
			let output = ctx.run_ui(
				egui::RawInput {
					screen_rect: Some(egui::Rect::from_min_size(
						egui::Pos2::ZERO,
						egui::vec2(size[0], size[1]),
					)),
					..Default::default()
				},
				|ui| {
					let _ = messaging.show(ui, &mut state);
				},
			);
			assert!(!output.shapes.is_empty());
			output.drop_without_applying_deltas();
		}
	}
	println!("Offline update flow, preference compatibility, and settings rendering passed.");
}
/// One offline debug path through the shipped Wasm, reducer, and egui rows.
#[cfg(feature = "demo")]
fn demo_check_extensions() {
	let _ = extensions::demo_check_examples().expect("starter packages activate with consent");
	let mut state = test_support::demo_state();
	state.set_preserve_deleted_messages(true);
	let channel = state.selected.expect("demo conversation");
	state.timeline.clear();
	let mut message = test_support::message(600, channel);
	message.content = "A useful message stays readable".into();
	message.attachments.clear();
	message.embeds.clear();
	state.timeline.insert(message.clone(), true, false).unwrap();
	state.apply(Envelope {
		generation: state.generation,
		event: Event::Delete {
			channel,
			id: message.id,
		},
	});
	assert!(state.timeline.is_deleted(message.id));
	assert!(
		state.timeline.get(message.id).is_none(),
		"deleted messages cannot receive service actions"
	);
	assert_eq!(
		state.timeline.get_display(message.id).unwrap().content,
		message.content
	);
	state
		.timeline
		.insert(message.clone(), false, false)
		.unwrap();
	assert!(
		state.timeline.get(message.id).is_none(),
		"stale history cannot resurrect a deletion"
	);
	let ctx = egui::Context::default();
	ui::fonts::install(&ctx);
	ui::design::apply(&ctx);
	let mut messaging = ui::MessagingUi::default();
	let mut saw_deleted = false;
	let mut saw_author = false;
	let mut saw_avatar = false;
	for _ in 0..3 {
		let mut deleted_color = egui::Color32::TRANSPARENT;
		let frame = ctx.run_ui(
			egui::RawInput {
				screen_rect: Some(egui::Rect::from_min_size(
					egui::Pos2::ZERO,
					egui::vec2(1120.0, 760.0),
				)),
				..Default::default()
			},
			|ui| {
				deleted_color = ui::design::palette(ui).danger;
				let _ = messaging.show(ui, &mut state);
			},
		);
		let body_left = frame.shapes.iter().find_map(|shape| match &shape.shape {
			egui::Shape::Text(text) if text.galley.text() == message.content => Some(text.pos.x),
			_ => None,
		});
		for shape in &frame.shapes {
			if let egui::Shape::Text(text) = &shape.shape {
				assert!(
					!text
						.galley
						.text()
						.contains("Deleted - kept by Message delete protector")
				);
				if text.galley.text() == message.content {
					assert!(
						text.galley
							.job
							.sections
							.iter()
							.all(|section| section.format.color == deleted_color)
					);
					saw_deleted = true;
				}
				saw_author |= text.galley.text() == message.author.name
					&& body_left.is_some_and(|left| (text.pos.x - left).abs() < 0.1);
			}
			if let egui::Shape::Rect(image) = &shape.shape
				&& image.brush.is_some()
			{
				let rect = image.rect;
				saw_avatar |= (rect.width() - 40.0).abs() < 0.1
					&& (rect.height() - 40.0).abs() < 0.1
					&& body_left.is_some_and(|left| (rect.right() + 16.0 - left).abs() < 0.1);
			}
		}
		frame.drop_without_applying_deltas();
	}
	assert!(
		saw_deleted && saw_author && saw_avatar,
		"retained row renders red text ({saw_deleted}), author ({saw_author}), and a normal 40-pixel avatar ({saw_avatar})"
	);
	state.discard_preserved_deleted(message.id);
	assert!(
		state.timeline.get_display(message.id).is_none(),
		"local remove drops the retained payload"
	);
	state.set_preserve_deleted_messages(false);
	let next = test_support::message(601, channel);
	state.timeline.insert(next.clone(), true, false).unwrap();
	state.apply(Envelope {
		generation: state.generation,
		event: Event::Delete {
			channel,
			id: next.id,
		},
	});
	assert!(
		state.timeline.get_display(next.id).is_none(),
		"loaded deletes disappear with the extension disabled"
	);
	assert!(state.timeline.get(next.id).is_none());
	println!(
		"Extension debug check passed: starter packages, consent, retained deleted row, stale-history rejection and local remove."
	);
}

fn parse_frame_sample(demo: bool, value: &str) -> Result<(Duration, Duration), &'static str> {
	let invalid = "Use --demo --demo-friends --demo-frame-sample=WARMUP,SAMPLE (whole seconds, warmup 1..600, sample 1..600)";
	let (warmup, sample) = value
		.strip_prefix('=')
		.and_then(|value| value.split_once(','))
		.ok_or(invalid)?;
	let warmup = warmup.parse::<u64>().map_err(|_| invalid)?;
	let sample = sample.parse::<u64>().map_err(|_| invalid)?;
	if !demo || !(1..=600).contains(&warmup) || !(1..=600).contains(&sample) {
		return Err(invalid);
	}
	Ok((Duration::from_secs(warmup), Duration::from_secs(sample)))
}

struct FrameSample {
	ready: std::time::Instant,
	duration: Duration,
	started: Option<std::time::Instant>,
	complete: bool,
}

/// Opt-in aggregate callback wall time; excludes tessellation/presentation, not a CPU timer.
/// Sample mode emits two bounded records and never schedules repaints.
struct FrameMetrics {
	enabled: bool,
	started: Option<std::time::Instant>,
	sample: Option<FrameSample>,
	frames: u64,
	inputless: u64,
	viewport_focused: u64,
	search_focused: u64,
	viewport_size: Option<[f32; 2]>,
	pixels_per_point: f32,
	max_micros: u64,
	buckets: [u64; 8],
	reflows: (u64, u64),
}
impl Default for FrameMetrics {
	fn default() -> Self {
		Self::new(None)
	}
}
impl FrameMetrics {
	fn new(sample: Option<(Duration, Duration)>) -> Self {
		Self {
			enabled: sample.is_some()
				|| std::env::var_os("SEREIN_FRAME_DIAGNOSTICS").is_some_and(|v| v == "1"),
			started: None,
			sample: sample.map(|(warmup, duration)| FrameSample {
				ready: std::time::Instant::now() + warmup,
				duration,
				started: None,
				complete: false,
			}),
			frames: 0,
			inputless: 0,
			viewport_focused: 0,
			search_focused: 0,
			viewport_size: None,
			pixels_per_point: 0.0,
			max_micros: 0,
			buckets: [0; 8],
			reflows: (0, 0),
		}
	}
	fn sample_active_at(&mut self, now: std::time::Instant) -> bool {
		let viewport_size = self.viewport_size;
		let pixels_per_point = self.pixels_per_point;
		let Some(sample) = &mut self.sample else {
			return true;
		};
		if sample.complete || now < sample.ready {
			return false;
		}
		let started = *sample.started.get_or_insert_with(|| {
			Self::sample_record(serde_json::json!({
				"serein_frame_sample": "start",
				"viewport_size": viewport_size,
				"pixels_per_point": pixels_per_point,
			}));
			now
		});
		let elapsed = now.duration_since(started);
		if elapsed < sample.duration {
			return true;
		}
		sample.complete = true;
		Self::sample_record(serde_json::json!({
			"serein_frame_sample": "complete",
			"elapsed_ms": elapsed.as_millis(),
			"callbacks": self.frames,
			"without_input": self.inputless,
			"viewport_focused": self.viewport_focused,
			"search_focused": self.search_focused,
			"viewport_size": viewport_size,
			"pixels_per_point": pixels_per_point,
			"callback_wall_us_limits": [1000, 2000, 4000, 8000, 16000, 32000, 64000],
			"callback_wall_us_buckets": self.buckets,
			"max_callback_wall_us": self.max_micros,
		}));
		false
	}
	fn sample_record(record: serde_json::Value) {
		use std::io::Write;
		let mut output = std::io::stdout().lock();
		let _ = writeln!(output, "{record}");
		let _ = output.flush();
	}
	fn begin(&mut self, ctx: &egui::Context, search_focused: bool) {
		self.started = None;
		if !self.enabled {
			return;
		}
		if self.sample.is_some() {
			self.viewport_size = ctx.input(|i| {
				i.viewport()
					.inner_rect
					.map(|rect| [rect.width(), rect.height()])
			});
			self.pixels_per_point = ctx.pixels_per_point();
		}
		let now = std::time::Instant::now();
		if self.sample_active_at(now) {
			self.started = Some(now);
			self.inputless += u64::from(ctx.input(|i| i.events.is_empty()));
			self.viewport_focused += u64::from(ctx.input(|i| i.focused));
			self.search_focused += u64::from(search_focused);
		}
	}
	fn finish(&mut self) {
		if let Some(started) = self.started.take() {
			let micros = started.elapsed().as_micros();
			let bucket = [1000, 2000, 4000, 8000, 16000, 32000, 64000]
				.partition_point(|limit| *limit < micros);
			self.buckets[bucket] += 1;
			self.max_micros = self
				.max_micros
				.max(u64::try_from(micros).unwrap_or(u64::MAX));
			self.frames += 1;
		}
	}
}
impl Drop for FrameMetrics {
	fn drop(&mut self) {
		if self.enabled && self.sample.is_none() {
			use std::io::Write;
			let _ = writeln!(
				std::io::stderr(),
				// Preserve the legacy diagnostic label; elapsed callback time is wall time.
				"[Serein frames] callbacks={} without_input={} cpu_us_buckets(1000,2000,4000,8000,16000,32000,64000,above)={:?} reflows(total,consecutive)={:?}",
				self.frames,
				self.inputless,
				self.buckets,
				self.reflows
			);
		}
	}
}
/// Every session teardown ends the same way; only saved data and what follows differ.
#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum SessionEnd {
	#[default]
	Logout,
	Switch(model::Id),
	Add,
}
impl SessionEnd {
	/// Switching and adding keep this account's cached data and saved login.
	fn forgets(self) -> bool {
		self == Self::Logout
	}
}
struct Desktop {
	extensions: extension_bridge::Bridge,
	extension_close_pending: bool,
	login: Option<platform::LoginView>,
	captcha: captcha::Captcha,
	connection: Option<connection::Connection>,
	state: State,
	messaging: ui::MessagingUi,
	/// Last invite counter a local Rich Presence client published, so it opens exactly once.
	rpc_invite_seen: u64,
	pointer: pointer::Pointer,
	downloads: downloads::Downloads,
	audio: audio::Audio,
	video: video::Video,
	/// Offline fixture flags start (and optionally pause) the demo attachment without input.
	demo_video_autoplay: Option<bool>,
	notifications: platform::notifications::Notifications,
	notification_runtime: notification_runtime::Runtime,
	uploads: uploads::Uploads,
	interaction_files: interaction_uploads::Files,
	group_icon: group_icon::GroupIcon,
	create_server_icon: group_icon::GroupIcon,
	profile_avatar: group_icon::GroupIcon,
	server_icon: group_icon::GroupIcon,
	role_icon: group_icon::GroupIcon,
	role_icon_scope: Option<(u64, model::Id, model::Id, u64)>,
	emoji_upload: emoji_upload::EmojiUpload,
	sticker_upload: sticker_upload::StickerUpload,
	clipboard: Option<clipboard::Paste>,
	download_close_pending: bool,
	window: Arc<winit::window::Window>,
	monitor_geometry: Option<(Option<egui::Rect>, Option<f32>)>,
	monitor_period: Option<Duration>,
	frame_metrics: FrameMetrics,
	#[cfg(feature = "demo")]
	rendering_demo: Option<rendering_demo::RenderingDemo>,
	avatars: Option<avatars::AvatarWorker>,
	avatar_start_failed: bool,
	avatar_clear_account: Option<model::Id>,
	avatar_cleanup: Option<std::sync::mpsc::Receiver<Result<(), &'static str>>>,
	voice: voice::Voice,
	runtime: tokio::runtime::Runtime,
	store: Option<credentials::Store>,
	cache: Option<cache::Cache>,
	cache_pending: usize,
	cache_clears: cache::HistoryClears,
	cache_error: bool,
	cache_status: &'static str,
	appearance: egui::ThemePreference,
	appearance_changed: bool,
	transparency_available: bool,
	window_blur: Option<platform::window_effects::Blur>,
	window_transparent: bool,
	reading: reading_settings::ReadingSettings,
	app_settings: app_settings::Settings,
	font_picker: Option<std::sync::mpsc::Receiver<font_import::Selected>>,
	updater: updater::Updater,
	game_activity: toggle_setting::Settings,
	tray_setting: toggle_setting::Settings,
	startup: startup::Startup,
	tray: Option<platform::tray::Tray>,
	hotkeys: platform::hotkeys::Hotkeys,
	tray_error: Option<&'static str>,
	tray_window: tray_window::State,
	/// `--demo-reply`: keeps two synthetic typists active on the selected fixture channel.
	#[cfg(feature = "demo")]
	demo_typing: bool,
	variant_changed: bool,
	pending_save: Option<Arc<SessionSecret>>,
	/// Written under the account's own entry on every READY, including a launch restore, so
	/// the switcher can always get back to an account it lists.
	pending_account_save: Option<Arc<SessionSecret>>,
	account_presences: std::collections::BTreeMap<model::Id, model::OwnPresence>,
	presence_load_pending: bool,
	deferred_connect: Option<(SessionSecret, bool)>,
	presence_authoritative: bool,
	presence_saved: Option<(model::Id, model::OwnPresence)>,
	/// Saved account awaiting the owner's confirmation before it is forgotten.
	confirming_forget: Option<model::Id>,
	credential_status: &'static str,
	forgetting: bool,
	confirming_close: bool,
	confirming_logout: bool,
	/// What the pending session teardown is for: logging out, switching or adding an account.
	end_intent: SessionEnd,
	/// Saved account whose token is being read for a switch.
	switching: Option<model::Id>,
	/// A switcher-roster write is in flight; failures stop retrying for this session.
	roster_pending: bool,
	roster_failed: bool,
	/// Session generation whose account was last recorded in the switcher roster.
	roster_generation: Option<u64>,
	close_approved: bool,
	fixture_only: bool,
	authorized: bool,
	/// Sign-in screen disclosures, folded away until the owner opens them.
	about_open: bool,
	token_open: bool,
	/// Height the sign-in card took last frame, so a tall card stays fully reachable.
	sign_in_height: f32,
	#[cfg(feature = "demo")]
	synthetic_id: u64,
	token_input: Zeroizing<String>,
}
/// Check only navigation whose effective access can change with this event.
fn access_candidates(state: &State, event: &Event) -> Vec<model::Id> {
	use client_core::permissions::Event as Permission;
	let (guilds, channel): (Option<Vec<model::Id>>, Option<model::Id>) = match event {
		Event::Startup(_) | Event::Ready { .. } | Event::Permissions(Permission::Snapshot(_)) => {
			(None, None)
		}
		Event::Permissions(permission) => match permission {
			Permission::Guild(guild) => (Some(vec![guild.id]), None),
			Permission::Role { guild, .. }
			| Permission::RoleRemoved { guild, .. }
			| Permission::Member { guild, .. }
			| Permission::Owner { guild, .. }
			| Permission::UnavailableGuild(guild) => (Some(vec![*guild]), None),
			Permission::Members(members) => (Some(members.iter().map(|m| m.0).collect()), None),
			Permission::Channel { channel, .. } => (None, Some(*channel)),
			Permission::Snapshot(_) => unreachable!(),
		},
		Event::ChannelCreated(channel) | Event::ChannelRestored(channel) => {
			// A new channel cannot revoke existing access. Duplicate IDs can replace metadata.
			if state.channel(channel.id).is_none() {
				return Vec::new();
			}
			(None, Some(channel.id))
		}
		Event::ChannelChanged(patch) | Event::ThreadChanged { patch, .. } => (None, Some(patch.id)),
		Event::ThreadRemoved { id, .. } => (None, Some(*id)),
		Event::ChannelAction(client_core::channel_actions::Event::Finished {
			channel,
			result:
				Ok(
					client_core::channel_actions::Outcome::Deleted
					| client_core::channel_actions::Outcome::Channel { .. },
				),
			..
		}) => (None, Some(*channel)),
		Event::UserAction(client_core::user_actions::Event::Written {
			action: client_core::user_actions::Action::CloseDm(channel),
			result: Ok(()),
			..
		}) => (None, Some(*channel)),
		Event::GroupAction(client_core::group_actions::Event::Written {
			channel,
			result: Ok(None),
			..
		}) => (None, Some(*channel)),
		Event::ServerAction(client_core::server_actions::Event::Written {
			action: client_core::server_actions::Action::Leave(guild),
			result: Ok(None),
			..
		}) => (Some(vec![*guild]), None),
		Event::ThreadsSync { guild, .. } => (Some(vec![*guild]), None),
		_ => return Vec::new(),
	};
	state
		.channels
		.iter()
		.filter(|c| {
			c.supports_text()
				&& channel.is_none_or(|id| c.id == id || c.parent_id == Some(id))
				&& guilds
					.as_ref()
					.is_none_or(|ids| c.guild.is_some_and(|id| ids.contains(&id)))
		})
		.map(|c| c.id)
		.collect()
}
fn user_action_notice(event: &Event) -> Option<(ui::design::Level, &'static str)> {
	let Event::UserAction(client_core::user_actions::Event::Written { action, result, .. }) = event
	else {
		return None;
	};
	Some(match result {
		Ok(()) if matches!(action, client_core::user_actions::Action::OpenDm(_)) => {
			(ui::design::Level::Error, action.completion_label())
		}
		Ok(()) => (ui::design::Level::Success, action.completion_label()),
		Err(failure) => (ui::design::Level::Error, failure.label()),
	})
}
fn queue_channel_preferences(
	cache: Option<&cache::Cache>,
	messaging: &mut ui::MessagingUi,
	generation: u64,
	account: model::Id,
) -> bool {
	if !messaging.channel_preferences_reload || messaging.channel_preferences_load_pending {
		return false;
	}
	let Some(cache) = cache else {
		messaging.channel_preferences_reload = false;
		messaging.channel_preferences_status =
			"Local storage is unavailable; channel preferences could not be restored.";
		return false;
	};
	let accepted = cache.queue(
		generation,
		account,
		cache::Operation::LoadChannelPreferences,
	);
	// Keep one request until the bounded worker has room. Its completions wake the UI;
	// queue pressure is not a failed read and needs neither a timer nor another click.
	messaging.channel_preferences_reload = !accepted;
	messaging.channel_preferences_load_pending = accepted;
	if accepted {
		messaging.channel_preferences_status = "";
	}
	accepted
}

fn wants_cached_history(state: &State, channel: model::Id, request: u64) -> bool {
	state.selected == Some(channel)
		&& state.request == request
		&& state.history_pending
		&& state.freshness == model::Freshness::Loading
		&& state.timeline.row_count() == 0
		&& state.can_read_history(channel)
		&& state
			.channels
			.iter()
			.any(|c| c.id == channel && c.supports_text())
}
fn hydrate_cached_history(
	state: &mut State,
	channel: model::Id,
	request: u64,
	messages: Vec<model::Message>,
) {
	if wants_cached_history(state, channel, request)
		&& messages.iter().all(|message| message.channel == channel)
		&& state.timeline.seed_cache(messages).is_ok()
	{
		state.revision += 1;
	}
	state.enforce_resident_budget();
}
fn hydrate_cache_result(state: &mut State, safety: &cache::HistorySafety, outcome: cache::Outcome) {
	if let cache::Outcome::Channel {
		channel,
		request,
		messages,
		epoch,
	} = outcome
		&& safety.allows(epoch)
	{
		hydrate_cached_history(state, channel, request, messages);
	}
}

/// Soft radial accent glow behind the pre-session screens instead of a flat canvas.
fn accent_glow(ui: &egui::Ui) {
	let accent = ui::design::palette(ui).accent;
	let rect = ui.max_rect();
	let glow = rect.center() - egui::vec2(0.0, rect.height() * 0.1);
	let radius = rect.width().max(rect.height()) * 0.55;
	let mut mesh = egui::Mesh::default();
	let alpha = if ui.visuals().dark_mode { 0.22 } else { 0.12 };
	mesh.colored_vertex(glow, accent.gamma_multiply(alpha));
	const SEGMENTS: u32 = 48;
	for i in 0..=SEGMENTS {
		let angle = i as f32 / SEGMENTS as f32 * std::f32::consts::TAU;
		mesh.colored_vertex(
			glow + egui::vec2(angle.cos(), angle.sin()) * radius,
			egui::Color32::TRANSPARENT,
		);
	}
	for i in 1..=SEGMENTS {
		mesh.add_triangle(0, i, i + 1);
	}
	ui.painter().add(egui::Shape::mesh(mesh));
}
fn recovery_draft(state: &State, channel: model::Id) -> String {
	state
		.drafts
		.get(&channel)
		.filter(|text| !text.is_empty())
		.cloned()
		.or_else(|| {
			state
				.pending
				.iter()
				.rev()
				.find(|pending| {
					pending.channel == channel && pending.delivery != Delivery::Confirmed
				})
				.map(|pending| pending.content.clone())
		})
		.unwrap_or_default()
}
fn confirmed_recovery_channel(state: &State, event: &Event) -> Option<model::Id> {
	let (message, nonce) = match event {
		Event::SendResult {
			result: Ok(message),
			nonce,
		} => (message, nonce.as_str()),
		Event::Message(message) => (message, message.nonce.as_deref()?),
		_ => return None,
	};
	if state.user.as_ref().map(|user| user.id) != Some(message.author.id) {
		return None;
	}
	state
		.pending
		.iter()
		.any(|pending| pending.channel == message.channel && pending.nonce == nonce)
		.then_some(message.channel)
}
fn changes_active_history(state: &State, event: &Event) -> bool {
	let channel = match event {
		Event::History {
			channel, request, ..
		} if *request == state.request && state.history_pending => channel,
		Event::Message(message)
		| Event::SendResult {
			result: Ok(message),
			..
		} => &message.channel,
		Event::ServerAction(client_core::server_actions::Event::InviteSent {
			result: Ok(sent),
			..
		}) => &sent.1.channel,
		Event::Patch(patch) => &patch.channel,
		Event::Edited { channel, .. }
		| Event::Delete { channel, .. }
		| Event::DeleteBulk { channel, .. } => channel,
		_ => return false,
	};
	state.selected == Some(*channel)
}
/// Synthetic People rows with presence; never a Discord member directory.
#[cfg(feature = "demo")]
fn demo_members(guild: Option<model::Id>, channel: model::Id, request: u64) -> model::MemberList {
	let mut members = vec![
		model::Member {
			user: test_support::message(2, channel).author,
			nick: None,
			roles: if guild.is_some() {
				vec![model::Id(9001)]
			} else {
				vec![]
			},
			status: Some("idle".into()),
			custom_status: None,
			clients: model::ClientPlatforms::default(),
			activities: vec![],
		},
		model::Member {
			user: test_support::message(1, channel).author,
			nick: None,
			roles: if guild.is_some() {
				vec![model::Id(9002)]
			} else {
				vec![]
			},
			status: Some("online".into()),
			custom_status: Some("🌙 semifluent in synthetic data".into()),
			clients: model::ClientPlatforms::default(),
			activities: vec![model::RichActivity {
				kind: 0,
				name: "Stardew Valley".into(),
				details: Some("Tending the synthetic farm".into()),
				state: Some("Spring - Day 12".into()),
				image: Some(model::ActivityImage::Asset {
					application: model::Id(9001),
					asset: model::Id(9002),
				}),
				small_image: None,
				ends_at: None,
				started_at: None,
			}],
		},
	];
	if guild.is_some() {
		for (id, name, status) in [
			(9003, "Alex (synthetic)", "online"),
			(9004, "Sam (synthetic)", "offline"),
		] {
			let mut member = members[0].clone();
			member.user.id = model::Id(id);
			member.user.name = name.into();
			member.roles.clear();
			member.status = Some(status.into());
			members.push(member);
		}
	}
	model::MemberList {
		guild,
		channel,
		request,
		total: members.len() as u64,
		start: 0,
		slots: members
			.into_iter()
			.map(|m| Some(model::MemberSlot::Person(m)))
			.collect(),
		lazy: false,
		groups: vec![],
		ranges: vec![],
		freshness: model::Freshness::Fresh,
	}
}

#[cfg(target_os = "windows")]
fn align_undecorated_surface(window: &winit::window::Window) {
	use winit::platform::windows::WindowExtWindows as _;
	// egui-winit turns on winit's 1px restored-client shift for custom chrome.
	// Maximized skips the shift.
	window.set_undecorated_shadow(false);
}

impl Desktop {
	fn new(
		cc: &eframe::CreationContext<'_>,
		demo: bool,
		frame_sample: Option<(Duration, Duration)>,
		transparency_available: bool,
	) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
		ui::fonts::install(&cc.egui_ctx);
		ui::emoji::install(&cc.egui_ctx)?;
		ui::icons::install(&cc.egui_ctx);
		#[cfg(feature = "demo")]
		if demo {
			// Fixture-only preset preview, e.g. `--demo --demo-theme=onyx --demo-light`.
			if let Some(variant) = std::env::args()
				.find_map(|arg| arg.strip_prefix("--demo-theme=").map(str::to_owned))
				.and_then(|key| ui::design::Variant::from_key(&key))
			{
				ui::design::set_variant(variant);
			}
		}
		ui::design::apply(&cc.egui_ctx);
		cc.egui_ctx.set_theme(
			if demo && std::env::args().any(|arg| arg == "--demo-light") {
				egui::ThemePreference::Light
			} else {
				egui::ThemePreference::System
			},
		);
		let runtime = tokio::runtime::Builder::new_multi_thread()
			.worker_threads(2)
			.enable_all()
			.build()?;
		let mut store = (!demo).then(|| credentials::Store::start(cc.egui_ctx.clone()));
		let cache = (!demo).then(|| cache::Cache::start(cc.egui_ctx.clone()));
		#[cfg(all(debug_assertions, feature = "demo"))]
		if demo && std::env::args().any(|arg| arg == "--demo-voice-messages") {
			audio::debug_voice_message_check();
		}
		let mut state = State::default();
		#[cfg(feature = "demo")]
		if demo {
			state = {
				if std::env::args().any(|arg| arg == "--demo-slash-commands") {
					slash_demo::preview()
				} else if std::env::args().any(|arg| arg == "--demo-components") {
					components_demo::preview()
				} else if std::env::args().any(|arg| arg == "--demo-forwarded") {
					test_support::forwarded_demo_state()
				} else if std::env::args()
					.any(|arg| arg == "--demo-audio" || arg == "--demo-voice-messages")
				{
					test_support::audio_demo_state()
				} else if std::env::args().any(|arg| {
					matches!(
						arg.as_str(),
						"--demo-video" | "--demo-video-playing" | "--demo-video-paused"
					)
				}) {
					test_support::video_demo_state()
				} else if std::env::args().any(|arg| arg == "--demo-friends") {
					test_support::friends_demo_state()
				} else if std::env::args().any(|arg| arg == "--demo-system-messages") {
					test_support::system_demo_state()
				} else if std::env::args().any(|arg| arg == "--demo-code") {
					test_support::code_demo_state()
				} else if std::env::args().any(|arg| arg == "--demo-notifications") {
					test_support::notification_demo_state()
				} else if std::env::args().any(|arg| {
					matches!(
						arg.as_str(),
						"--demo-voice" | "--demo-voice-failed" | "--demo-voice-video"
					)
				}) {
					test_support::voice_demo_state()
				} else if std::env::args().any(|arg| arg == "--demo-existing-call") {
					test_support::existing_call_demo_state()
				} else if std::env::args()
					.any(|arg| matches!(arg.as_str(), "--demo-call" | "--demo-call-stream"))
				{
					test_support::call_demo_state()
				} else if std::env::args().any(|arg| {
					matches!(
						arg.as_str(),
						"--demo-empty-channel" | "--demo-empty-channel-long"
					)
				}) {
					test_support::empty_channel_demo_state(
						std::env::args().any(|arg| arg == "--demo-empty-channel-long"),
					)
				} else if std::env::args().any(|arg| arg == "--demo-chat") {
					test_support::chat_demo_state()
				} else {
					let mut state = test_support::demo_state();
					test_support::seed_demo_folder_mosaic(&mut state);
					test_support::seed_access_marks(&mut state);
					state
				}
			};
		}
		#[cfg(feature = "demo")]
		if demo && std::env::args().any(|arg| arg == "--demo-voice-failed") {
			let call = state
				.voice
				.active
				.as_ref()
				.expect("voice fixture has a call");
			let (channel, request) = (call.channel, call.request);
			let reason = "Audio device stopped or disconnected; choose a device and call again";
			state.apply_voice(client_core::voice::Event::Failed {
				channel,
				request,
				message: reason,
			});
			// Synthetic recovery events must not erase the reason before it can be copied.
			state.disconnect_voice("Later gateway disconnect");
			state.apply_voice(client_core::voice::Event::Deleted { channel });
			state.apply_voice(client_core::voice::Event::Progress {
				channel,
				request,
				phase: client_core::voice::Phase::Connected,
			});
			let call = state
				.voice
				.active
				.as_ref()
				.expect("failure survives departure");
			assert_eq!(call.error, Some(reason));
			assert_eq!(call.phase, client_core::voice::Phase::Failed);
		}
		#[cfg(feature = "demo")]
		if demo {
			if !std::env::args().any(|arg| arg == "--demo-friends") {
				let fixture = demo_members(None, model::Id(22), 0);
				state.direct_presences = fixture
					.slots
					.into_iter()
					.flatten()
					.filter_map(|slot| match slot {
						model::MemberSlot::Person(member) => Some(member),
						_ => None,
					})
					.filter(|member| member.user.id != model::Id(1))
					.map(|member| model::MemberPresence {
						user: member.user.id,
						status: member.status,
						custom_status: member.custom_status,
						clients: member.clients,
						activities: member.activities,
					})
					.collect();
			}
			// Synthetic role metadata exercises the same bounded permission mirror as live events.
			for guild in state.permissions.guilds.values_mut() {
				if let Some(roles) = &mut guild.roles {
					roles.extend([
						model::permissions::Role {
							id: model::Id(9001),
							bits: 0,
							name: "Founders".into(),
							color: 0xe78284,
							position: 2,
							hoist: true,
						},
						model::permissions::Role {
							id: model::Id(9002),
							bits: 0,
							name: "Community".into(),
							color: 0xe5c769,
							position: 1,
							hoist: true,
						},
					]);
				}
			}
		}
		let loading_saved = store
			.as_mut()
			.is_some_and(|store| store.load(state.generation, std::time::Instant::now()));
		let mut cache_pending = usize::from(cache.as_ref().is_some_and(|cache| {
			// Appearance has its own singleton table; this account ID is unused.
			cache.queue(
				state.generation,
				model::Id(0),
				cache::Operation::LoadAppearance,
			)
		}));
		if let Some(cache) = &cache
			&& cache.queue(
				state.generation,
				model::Id(0),
				cache::Operation::LoadAccounts,
			) {
			cache_pending += 1;
		}
		let presence_load_pending = cache.as_ref().is_some_and(|cache| {
			cache.queue(
				state.generation,
				model::Id(0),
				cache::Operation::LoadAccountPresences,
			)
		});
		cache_pending += usize::from(presence_load_pending);
		let mut app_settings = app_settings::Settings::default();
		if cache.as_ref().is_some_and(|cache| {
			cache.queue(
				state.generation,
				model::Id(0),
				cache::Operation::LoadAppPreferences,
			)
		}) {
			cache_pending += 1;
		} else if !demo {
			app_settings.state.failed = true;
		}
		let mut reading = reading_settings::ReadingSettings::default();
		let mut game_activity = toggle_setting::Settings::default();
		let mut tray_setting = toggle_setting::Settings::with_default(true);
		if cache.as_ref().is_some_and(|cache| {
			cache.queue(
				state.generation,
				model::Id(0),
				cache::Operation::LoadMinimizeToTray,
			)
		}) {
			cache_pending += 1;
		} else if !demo {
			tray_setting.failed = true;
		}
		if cache.as_ref().is_some_and(|cache| {
			cache.queue(
				state.generation,
				model::Id(0),
				cache::Operation::LoadGameActivity,
			)
		}) {
			cache_pending += 1;
		} else if !demo {
			game_activity.failed = true;
		}
		if let Some(cache) = &cache {
			if cache.queue(
				state.generation,
				model::Id(0),
				cache::Operation::LoadReadingPreferences,
			) {
				cache_pending += 1;
			} else {
				reading.restore(Err(local_store::StoreError::Unavailable));
			}
		}
		#[cfg(feature = "demo")]
		let synthetic_id = state
			.timeline
			.iter()
			.last()
			.map_or(10_000, |m| m.id.0.max(10_000));
		let mut messaging = ui::MessagingUi::default();
		if !demo {
			messaging.custom_font.busy = cache.as_ref().is_some_and(|cache| {
				cache.queue(
					state.generation,
					model::Id(0),
					cache::Operation::LoadCustomFont,
				)
			});
			cache_pending += usize::from(messaging.custom_font.busy);
			messaging.custom_font.status = if messaging.custom_font.busy {
				"Loading saved font…"
			} else {
				"Could not load the saved font."
			};
		}
		messaging.minimize_to_tray = tray_setting.enabled;
		let preference_defaults = local_store::AppPreferences::default();
		messaging.notifications_enabled = preference_defaults.notifications_enabled;
		messaging.transparency = preference_defaults.transparency;
		messaging.blur = preference_defaults.blur;
		#[cfg(feature = "demo")]
		if demo {
			messaging.transparency_blur = transparency_available;
			// Robin stays pinned on home. #getting-started is the guild Favorites row.
			let _ = messaging
				.channel_preferences
				.set(model::Shortcut::Pinned, model::Id(22), true);
			let _ =
				messaging
					.channel_preferences
					.set(model::Shortcut::Favorite, model::Id(20), true);
			messaging.show_hidden_channels = true;
		}
		#[cfg(feature = "demo")]
		if frame_sample.is_some() {
			messaging.prepare_friends_sample();
		}
		messaging.tray_available = platform::tray::supported();
		// Name the adapter actually in use; a mismatch with the preference is the useful
		// detail in a graphics bug report.
		if let Some(render_state) = cc.wgpu_render_state.as_ref() {
			messaging.gpu_adapter = gpu::describe(&render_state.adapter.get_info());
		}
		let startup = startup::Startup::new(&cc.egui_ctx, &runtime, &mut messaging, demo);
		// `--demo-update`: a pending release without any network check. The system title bar
		// comes with it, since that is when the sidebar prompt stands in for the title strip.
		#[cfg(feature = "demo")]
		if demo && std::env::args().any(|arg| arg == "--demo-update") {
			messaging.hide_title_bar = true;
			// The demo updater owns the flags, so ask it for its synthetic release.
			messaging.updates.check_requested = true;
		}
		// `--demo-toast`: representative semantic colors, so the transient notice layer
		// can be captured without provoking a real failure.
		#[cfg(feature = "demo")]
		if demo && std::env::args().any(|arg| arg == "--demo-toast") {
			messaging.toasts.push(
				ui::design::Level::Error,
				"Attach up to 10 files per message",
			);
			messaging.toasts.push(
				ui::design::Level::Warning,
				"Attachments must total at most 500 MB; account limits may be lower",
			);
			messaging
				.toasts
				.push(ui::design::Level::Success, "Friend request sent");
		}
		#[cfg(feature = "demo")]
		if demo && std::env::args().any(|arg| arg == "--demo-game-activity") {
			messaging.share_game_activity = true;
			messaging.own_game = Some("Playing osu!".into());
		}
		// `--demo-call-stream`: the direct-message peer shares a synthetic screen this device is
		// watching, so the stream stage renders offline without any capture or network.
		#[cfg(feature = "demo")]
		if demo && std::env::args().any(|arg| arg == "--demo-call-stream") {
			let peer = model::Id(2);
			if let Some(call) = &mut state.voice.active {
				for participant in &mut call.participants {
					if participant.user == peer {
						participant.streaming = true;
					}
				}
			}
			let _ = state.watch_stream(peer);
			let (width, height) = (640usize, 360usize);
			let pixels = (0..width * height)
				.map(|i| {
					let (x, y) = (i % width, i / width);
					let grid = usize::from(x % 80 < 2 || y % 80 < 2) as u8;
					egui::Color32::from_rgb(
						24 + grid * 40 + (x * 90 / width) as u8,
						28 + grid * 40 + (y * 70 / height) as u8,
						48 + grid * 50,
					)
				})
				.collect();
			let image = egui::ColorImage {
				size: [width, height],
				source_size: egui::vec2(width as f32, height as f32),
				pixels,
			};
			messaging.voice_stream_view = Some(cc.egui_ctx.load_texture(
				"synthetic-stream",
				image,
				egui::TextureOptions::LINEAR,
			));
			messaging.voice_stream_status = "Watching the stream";
			// `--demo-focus` additionally opens the enlarged stage layout.
			if std::env::args().any(|arg| arg == "--demo-focus") {
				messaging.voice_focus = Some(ui::StageFocus::Stream(peer));
			}
		}
		// `--demo-voice-video`: Robin's camera is a synthetic gradient and starts enlarged, so the
		// focused stage layout renders offline without any capture or network.
		#[cfg(feature = "demo")]
		if demo && std::env::args().any(|arg| arg == "--demo-voice-video") {
			let robin = model::Id(2);
			for entry in &mut state.voice.roster {
				if entry.participant.user == robin {
					entry.participant.video = true;
				}
			}
			if let Some(call) = &mut state.voice.active {
				for participant in &mut call.participants {
					if participant.user == robin {
						participant.video = true;
					}
				}
			}
			let (width, height) = (320usize, 240usize);
			let pixels = (0..width * height)
				.map(|i| {
					let (x, y) = (
						(i % width) as f32 / width as f32,
						(i / width) as f32 / height as f32,
					);
					egui::Color32::from_rgb((90.0 + 120.0 * x) as u8, (60.0 + 90.0 * y) as u8, 140)
				})
				.collect();
			let image = egui::ColorImage {
				size: [width, height],
				source_size: egui::vec2(width as f32, height as f32),
				pixels,
			};
			messaging.voice_remote_video.push((
				robin,
				cc.egui_ctx.load_texture(
					"synthetic-remote-camera",
					image,
					egui::TextureOptions::LINEAR,
				),
			));
			messaging.voice_focus = Some(ui::StageFocus::Participant(robin));
		}
		messaging.build = ui::design::Build {
			channel: if cfg!(debug_assertions) {
				ui::design::Channel::Dev
			} else if option_env!("SEREIN_CHANNEL") == Some("nightly") {
				ui::design::Channel::Nightly
			} else {
				ui::design::Channel::Stable
			},
			version: env!("CARGO_PKG_VERSION"),
		};
		messaging.notification_test_available =
			demo && std::env::args().any(|arg| arg == "--demo-system-notifications");
		if messaging.notification_test_available {
			state.status = "Offline fixture · explicit system notification test";
		}
		#[cfg(feature = "demo")]
		if demo
			&& let Some(page) = std::env::args().find_map(|arg| {
				arg.strip_prefix("--demo-settings")
					.map(|rest| rest.trim_start_matches('=').to_lowercase())
			}) {
			// `--demo-settings` or `--demo-settings=account` etc.
			messaging.preview_settings(&page);
		}
		#[cfg(feature = "demo")]
		if demo
			&& let Some(tab) = std::env::args().find_map(|arg| {
				arg.strip_prefix("--demo-theme-maker")
					.map(|rest| rest.trim_start_matches('=').to_owned())
			}) {
			// `--demo-theme-maker` or `--demo-theme-maker=advanced`.
			messaging.preview_theme_maker(&tab);
		}
		#[cfg(feature = "demo")]
		if demo
			&& std::env::args().any(|arg| arg == "--demo-threads")
			&& let Some(selected) = state.selected
		{
			// Threads dialog for the fixture channel, for screenshots.
			messaging.preview_threads(selected);
		}
		#[cfg(feature = "demo")]
		if demo
			&& std::env::args().any(|arg| arg == "--demo-thread-view")
			&& let Some(thread) = state
				.channels
				.iter()
				.find(|c| {
					c.guild.is_some()
						&& matches!(c.kind, 10..=12)
						&& c.parent_id
							.and_then(|id| state.channels.iter().find(|p| p.id == id))
							.is_some_and(|p| matches!(p.kind, 0 | 5))
				})
				.map(|c| c.id)
		{
			// Open the fixture thread itself, so its starter message renders for screenshots.
			let _ = state.select(thread);
			// A short synthetic thread: its whole history fits, so the starter sits at the top.
			for id in [thread.0 + 10, thread.0 + 20, thread.0 + 30] {
				let _ = state
					.timeline
					.insert(test_support::message(id, thread), false, false);
			}
			state.history_pending = false;
			state.freshness = model::Freshness::Fresh;
			state.older_exhausted = true;
		}
		#[cfg(feature = "demo")]
		if demo && std::env::args().any(|arg| arg == "--demo-profile") {
			// Presence for the fixture card comes from the same synthetic People rows.
			let _ = state.request_members();
			if let Some(list) = &state.members {
				state.members = Some(demo_members(list.guild, list.channel, list.request));
			}
			let user = if messaging.share_game_activity {
				state.user.clone().expect("demo has a current user")
			} else {
				test_support::message(1, model::Id(20)).author
			};
			messaging.preview_profile(user);
			state.status = "Offline fixture · synthetic profile card opened at startup";
		}
		#[cfg(feature = "demo")]
		let demo_typing = demo && std::env::args().any(|arg| arg == "--demo-reply");
		#[cfg(feature = "demo")]
		if demo_typing {
			// Reply bar plus an active typing row on the fixture conversation, for screenshots.
			state.reply = state
				.timeline
				.iter()
				.last()
				.map(|message| client_core::Reply::to(message.id));
			state.status = "Offline fixture · reply bar and typing row shown at startup";
		}
		#[cfg(feature = "demo")]
		if demo && std::env::args().any(|arg| arg == "--demo-pins") {
			messaging.preview_pins();
			state.status = "Offline fixture · pinned messages popout opened at startup";
		}
		#[cfg(feature = "demo")]
		if demo
			&& let Some(rest) = std::env::args().find_map(|arg| {
				arg.strip_prefix("--demo-account")
					.map(|rest| rest.trim_start_matches('=').to_owned())
			}) {
			// `--demo-account` or `--demo-account=status` for the written-status variant.
			if let Some(Command::EditProfile { user, request, .. }) = state.load_own_profile() {
				let profile = ui::synthetic_own_profile(state.user.as_ref().expect("demo user"));
				state.apply(client_core::Envelope {
					generation: state.generation,
					event: Event::ProfileEdited {
						user,
						request,
						result: Ok(Box::new(profile)),
					},
				});
			}
			if rest == "status" {
				messaging.own_presence.custom_status = "Shipping a nicer popout".into();
			}
			if let Some(user) = state.user.as_ref() {
				messaging.accounts = test_support::demo_accounts(user);
			}
			if rest == "status-editor" {
				messaging.own_presence.custom_status = "Shipping a nicer popout".into();
				messaging.preview_custom_status(state.generation);
			} else {
				messaging.preview_account_menu(state.generation);
			}
			state.status = "Offline fixture · account popout opened at startup";
		}
		#[cfg(feature = "demo")]
		if demo && std::env::args().any(|arg| arg == "--demo-stickers") {
			test_support::seed_stickers(&mut state);
			messaging.preview_sticker_picker();
			state.status = "Offline fixture: sticker picker";
		}
		#[cfg(feature = "demo")]
		if demo && std::env::args().any(|arg| arg == "--demo-emoji") {
			messaging.preview_emoji_picker();
			state.status = "Offline fixture · emoji popout opened at startup";
		}
		#[cfg(feature = "demo")]
		if demo && std::env::args().any(|arg| arg == "--demo-viewer") {
			// Opens the full-window media viewer on the fixture gallery message.
			messaging.preview_image_viewer(model::Id(500), model::Id(700));
			state.status = "Offline fixture · media viewer opened at startup";
		}
		#[cfg(feature = "demo")]
		if demo && std::env::args().any(|arg| arg == "--demo-captcha") {
			messaging.preview_verification(&mut state);
		}
		#[cfg(feature = "demo")]
		if demo && std::env::args().any(|arg| arg == "--demo-attachment=file") {
			// Non-image variant: exercises the file-kind glyph and extension badge.
			messaging.preview_attachment("quarterly-report.pdf", 1_482_311, None);
			state.status = "Offline fixture · synthetic file attachment staged in the composer";
		} else if demo
			&& std::env::args()
				.any(|arg| arg == "--demo-attachment" || arg == "--demo-attachment=multi")
		{
			// Synthetic gradients stand in for decoded photos; no file is read or uploaded.
			let gradient = |width: usize, height: usize, hue: f32| {
				let pixels = (0..width * height)
					.map(|index| {
						let (x, y) = (
							(index % width) as f32 / width as f32,
							(index / width) as f32 / height as f32,
						);
						let ring = ((x - 0.65).powi(2) + (y - 0.4).powi(2)).sqrt();
						if ring < 0.12 {
							egui::Color32::from_rgb(255, 214, 102)
						} else {
							egui::Color32::from_rgb(
								(40.0 + 120.0 * y * hue) as u8,
								(110.0 + 90.0 * x) as u8,
								(190.0 - 60.0 * y / hue) as u8,
							)
						}
					})
					.collect();
				egui::ColorImage {
					size: [width, height],
					source_size: egui::vec2(width as f32, height as f32),
					pixels,
				}
			};
			messaging.preview_attachment(
				"synthetic-holiday.png",
				2_437_120,
				Some(gradient(320, 200, 1.0)),
			);
			if std::env::args().any(|arg| arg == "--demo-attachment=multi") {
				// Batch variant: a portrait photo plus a document, like dropping three files.
				messaging.preview_attachment(
					"synthetic-portrait.png",
					1_106_944,
					Some(gradient(180, 320, 1.6)),
				);
				messaging.preview_attachment("synthetic-agenda.pdf", 482_311, None);
				state.status =
					"Offline fixture · three synthetic attachments staged in the composer";
			} else {
				state.status = "Offline fixture · synthetic attachment staged in the composer";
			}
		}
		#[cfg(feature = "demo")]
		if demo && std::env::args().any(|arg| arg == "--demo-sending") {
			messaging.preview_sending(&cc.egui_ctx, &mut state);
			state.status = "Offline fixture · synthetic pending message; no upload or send";
		}
		// `--demo-gifs`, `--demo-gifs=favorites`, `--demo-gifs=trending` or `--demo-gifs=<query>`.
		#[cfg(feature = "demo")]
		if demo
			&& let Some(section) = std::env::args().find_map(|arg| {
				arg.strip_prefix("--demo-gifs")
					.map(|rest| rest.strip_prefix('=').unwrap_or("").to_owned())
			}) {
			let favorites: Vec<_> = test_support::gif_page(None)
				.gifs
				.into_iter()
				.skip(2)
				.take(5)
				.collect();
			state.restore_gif_favorites(favorites);
			messaging.preview_gif_picker(&section);
			state.status = "Offline fixture · GIF popout opened at startup";
		}
		#[cfg(feature = "demo")]
		if demo
			&& let Some(query) = std::env::args()
				.find_map(|arg| arg.strip_prefix("--demo-search=").map(str::to_owned))
		{
			messaging.preview_search(&query);
			state.status = "Offline fixture · synthetic search opened at startup";
		}
		#[cfg(feature = "demo")]
		if demo
			&& std::env::args().any(|arg| arg == "--demo-browsing")
			&& let Some(channel) = state.selected
		{
			// Fixture-only: an unread marker on the oldest loaded message plus a targeted history
			// page, so both timeline overlays render without pointer input.
			let oldest = state.timeline.iter().next().map(|message| message.id);
			let _ = state.apply_read_state(client_core::read_state::Event::Snapshot {
				entries: Some(vec![(channel, oldest, 0)]),
				version: Some(1),
				partial: false,
			});
			state.history_targeted = true;
			state.status = "Offline fixture · unread strip and older-messages bar shown";
		}
		// Owner authorization for a new sign-in; fixture captures pre-tick it.
		#[cfg_attr(not(feature = "demo"), allow(unused_mut))]
		let mut authorized = false;
		#[cfg_attr(not(feature = "demo"), allow(unused_mut))]
		let mut sign_in_panels = false;
		#[cfg_attr(not(feature = "demo"), allow(unused_mut))]
		let mut sign_in_forget = None;
		#[cfg(feature = "demo")]
		if demo && std::env::args().any(|arg| arg == "--demo-join-server") {
			messaging.preview_join_server(state.generation);
			state.status = "Offline fixture · join-server dialog opened at startup";
		}
		#[cfg(feature = "demo")]
		if demo && std::env::args().any(|arg| arg == "--demo-screen-share") {
			// Pairs with `--demo-call`: synthetic sources, never a real capture.
			messaging.preview_screen_share(&state);
			state.status = "Offline fixture · screen-share picker opened at startup";
		}
		#[cfg(feature = "demo")]
		if demo
			&& let Some(rest) = std::env::args().find_map(|arg| {
				arg.strip_prefix("--demo-login")
					.map(|rest| rest.trim_start_matches('=').to_owned())
			}) {
			// Fixture-only: render the sign-in screen without a session. `--demo-login` shows the
			// returning-owner layout with a synthetic roster and pre-ticked consent (the actions
			// stay inert); `=new` shows the first-run layout, `=panels` the opened disclosures
			// and `=failed` the attention banner.
			if !matches!(rest.as_str(), "new" | "panels") {
				messaging.accounts = state
					.user
					.as_ref()
					.map(test_support::demo_accounts)
					.unwrap_or_default();
			}
			authorized = true;
			state.user = None;
			state.status = "Disconnected";
			sign_in_panels = rest == "panels";
			if rest == "forget" {
				sign_in_forget = messaging.accounts.get(1).map(|account| account.id);
			}
			if rest == "failed" {
				state.auth = AuthState::Failed;
				state.status = "Synthetic fixture failure · Discord was not contacted";
			}
		}
		#[cfg(feature = "demo")]
		if demo && std::env::args().any(|arg| arg == "--demo-server-settings") {
			server_settings_demo::open(&mut state, &mut messaging);
		}
		let mut hotkeys = platform::hotkeys::Hotkeys::new({
			let ctx = cc.egui_ctx.clone();
			move || ctx.request_repaint()
		});
		if !demo {
			hotkeys.sync(&messaging.keybinds, &runtime);
		}
		let window = cc
			.winit_window()
			.ok_or("Native window unavailable")?
			.clone();
		messaging.hide_window_decorations = cfg!(target_os = "linux") && !window.is_decorated();
		app_settings.current.hide_window_decorations = messaging.hide_window_decorations;
		#[cfg(target_os = "windows")]
		{
			use winit::platform::windows::{CornerPreference, WindowExtWindows as _};
			window.set_corner_preference(CornerPreference::Round);
			align_undecorated_surface(&window);
		}
		// The GPU surface and X11 visual are selected at startup. Opaque launches
		// keep the same native/compositor path as builds without window effects.
		let tray_window = tray_window::State::default();
		Ok(Self {
			extensions: extension_bridge::Bridge::default(),
			extension_close_pending: false,
			login: None,
			captcha: captcha::Captcha::default(),
			connection: None,
			state,
			messaging,
			rpc_invite_seen: 0,
			pointer: pointer::Pointer::default(),
			downloads: downloads::Downloads::default(),
			audio: audio::Audio::default(),
			video: video::Video::default(),
			demo_video_autoplay: if std::env::args().any(|arg| arg == "--demo-video-paused") {
				Some(true)
			} else if std::env::args().any(|arg| arg == "--demo-video-playing") {
				Some(false)
			} else {
				None
			},
			notification_runtime: Default::default(),
			notifications: {
				let wake = cc.egui_ctx.clone();
				platform::notifications::Notifications::new(
					move || wake.request_repaint(),
					tray_window.restorer(),
				)
			},
			uploads: uploads::Uploads::default(),
			interaction_files: Default::default(),
			group_icon: group_icon::GroupIcon::default(),
			create_server_icon: group_icon::GroupIcon::default(),
			profile_avatar: group_icon::GroupIcon::default(),
			server_icon: group_icon::GroupIcon::default(),
			role_icon: group_icon::GroupIcon::default(),
			role_icon_scope: None,
			emoji_upload: emoji_upload::EmojiUpload::default(),
			sticker_upload: sticker_upload::StickerUpload::default(),
			clipboard: None,
			download_close_pending: false,
			window_blur: transparency_available
				.then(|| platform::window_effects::Blur::new(window.clone())),
			window,
			monitor_geometry: None,
			monitor_period: None,
			frame_metrics: FrameMetrics::new(frame_sample),
			#[cfg(feature = "demo")]
			rendering_demo: (demo && std::env::args().any(|arg| arg == "--demo-rendering"))
				.then(rendering_demo::RenderingDemo::default),
			avatars: None,
			avatar_cleanup: None,
			avatar_start_failed: false,
			avatar_clear_account: None,
			voice: voice::Voice::default(),
			runtime,
			store,
			cache,
			cache_pending,
			cache_clears: Default::default(),
			cache_error: false,
			cache_status: "Loading local appearance…",
			appearance: egui::ThemePreference::System,
			appearance_changed: false,
			transparency_available,
			window_transparent: transparency_available,
			reading,
			app_settings,
			font_picker: None,
			updater: updater::Updater::new(demo),
			game_activity,
			tray_setting,
			startup,
			tray: None,
			tray_window,
			hotkeys,
			tray_error: None,
			#[cfg(feature = "demo")]
			demo_typing,
			variant_changed: false,
			pending_save: None,
			pending_account_save: None,
			account_presences: std::collections::BTreeMap::new(),
			presence_load_pending,
			deferred_connect: None,
			presence_authoritative: false,
			presence_saved: None,
			confirming_forget: sign_in_forget,
			credential_status: if demo {
				"Fixture mode never opens the credential store or network"
			} else if loading_saved {
				"Checking saved login…"
			} else {
				"Saved login unavailable; could not start credential lookup"
			},
			forgetting: false,
			confirming_close: false,
			confirming_logout: false,
			end_intent: SessionEnd::Logout,
			switching: None,
			roster_pending: false,
			roster_failed: false,
			roster_generation: None,
			close_approved: false,
			fixture_only: demo,
			authorized,
			about_open: sign_in_panels,
			token_open: sign_in_panels,
			sign_in_height: 0.0,
			#[cfg(feature = "demo")]
			synthetic_id,
			token_input: Zeroizing::new(String::new()),
		})
	}
	fn connect(&mut self, secret: SessionSecret, save: bool, ctx: &egui::Context) {
		if self.presence_load_pending {
			self.deferred_connect = Some((secret, save));
			self.credential_status = "Connecting to Discord…";
			return;
		}
		self.role_icon.cancel();
		self.role_icon_scope = None;
		self.group_icon.cancel();
		self.create_server_icon.cancel();
		self.profile_avatar.cancel();
		self.server_icon.cancel();
		self.emoji_upload.cancel();
		self.sticker_upload.cancel();
		if let Some(store) = &mut self.store {
			store.cancel_load();
		}
		self.credential_status = if save {
			"Login will be saved after Discord connects"
		} else {
			"Connecting with the supplied session; saved login unchanged"
		};
		self.voice.stop();
		self.messaging.camera_test_requested = false;
		self.messaging.camera_test_texture = None;
		self.state
			.disconnect_voice("Discord login session changed; start a new call");
		self.uploads.cancel();
		self.login = None;
		self.connection = None;
		if let Some(worker) = self.avatars.take() {
			self.avatar_cleanup = Some(worker.shutdown());
		}
		self.avatar_start_failed = false;
		self.messaging.clear_avatars();
		self.state.generation += 1;
		self.messaging.channel_preferences = model::ChannelPreferences::default();
		self.messaging.channel_preferences_changed = false;
		self.messaging.channel_preferences_loaded = false;
		self.messaging.channel_preferences_load_pending = false;
		self.messaging.channel_preferences_reload = false;
		self.messaging.channel_preferences_save_pending = false;
		self.messaging.channel_preferences_status = "";
		self.presence_authoritative = false;
		self.presence_saved = None;
		let cached = self
			.state
			.user
			.as_ref()
			.and_then(|user| self.account_presences.get(&user.id).cloned());
		if let Some(cached) = cached {
			self.messaging.adopt_account_presence(cached);
		} else {
			self.messaging.own_presence = model::OwnPresence::default();
			self.messaging.own_presence_expires = None;
		}
		self.messaging.own_presence_changed = false;
		self.messaging.draft_restore_pending = false;
		self.state.auth = AuthState::Authenticating;
		self.state.status = "Connecting to Discord…";
		let secret = Arc::new(secret);
		self.pending_save = save.then(|| secret.clone());
		self.pending_account_save = Some(secret.clone());
		self.connection = Some(connection::Connection::start(
			self.runtime.handle(),
			secret,
			self.state.generation,
			self.state.user.as_ref().map(|u| u.id),
			self.account_presences.clone(),
			ctx.clone(),
		));
	}
	fn logout(&mut self, ctx: &egui::Context) {
		self.end_session(ctx, SessionEnd::Logout);
	}
	/// Ends the current session. Only logging out removes this account's local data and
	/// saved login; switching and adding keep both so the account stays in the switcher.
	fn end_session(&mut self, ctx: &egui::Context, intent: SessionEnd) {
		self.captcha.close();
		self.notification_runtime.clear(&self.window);
		self.messaging.image_sharing_enabled = false;
		self.messaging.image_share_requested = None;
		let extension_logout = self.extensions.logout(ctx);
		self.role_icon.cancel();
		self.role_icon_scope = None;
		self.group_icon.cancel();
		self.create_server_icon.cancel();
		self.profile_avatar.cancel();
		self.server_icon.cancel();
		self.emoji_upload.cancel();
		self.sticker_upload.cancel();
		self.notifications.clear();
		self.uploads.cancel();
		if let Some(store) = &mut self.store {
			store.cancel_load();
		}
		self.downloads.cancel();
		self.audio.stop();
		self.video.stop();
		self.voice.stop();
		self.messaging.camera_test_requested = false;
		self.messaging.camera_test_texture = None;
		let was_demo = self.state.demo;
		self.clear_avatars(ctx);
		self.login = None;
		self.connection = None;
		self.pending_save = None;
		self.pending_account_save = None;
		let old_account = self.state.user.as_ref().filter(|_| !was_demo).map(|u| u.id);
		self.switching = None;
		self.roster_pending = false;
		self.state.logout();
		if intent.forgets()
			&& let (Some(cache), Some(account)) = (&self.cache, old_account)
		{
			if cache.queue(self.state.generation, account, cache::Operation::Forget) {
				self.cache_pending += 1;
			} else {
				self.cache_error = true;
				self.cache_status = "Could not queue local account data removal";
			}
		}
		self.messaging.clear();
		if intent.forgets() {
			self.messaging
				.accounts
				.retain(|saved| Some(saved.id) != old_account);
		}
		if let Err(error) = extension_logout {
			self.messaging.extensions.status = error;
			self.cache_error = true;
			self.cache_status = "Extension account data removal could not be queued";
		}
		self.app_settings.apply(&mut self.messaging);
		self.messaging.share_game_activity = self.game_activity.enabled;
		ctx.memory_mut(|m| *m = egui::Memory::default());
		let _ = ui::emoji::install(ctx);
		ui::design::apply(ctx);
		ctx.set_theme(self.appearance);
		self.messaging
			.apply_reading_preferences(ctx, self.reading.current);
		ctx.clear_animations();
		self.token_input = Zeroizing::new(String::new());
		if !was_demo
			&& intent.forgets()
			&& let Some(store) = &self.store
		{
			self.forgetting = store
				.send
				.try_send((
					self.state.generation,
					credentials::Request::NONE,
					credentials::Operation::Forget,
				))
				.is_ok();
			if let Some(account) = old_account {
				let _ = store.send.try_send((
					self.state.generation,
					credentials::Request::NONE,
					credentials::Operation::ForgetAccount(account),
				));
			}
			self.credential_status = if self.forgetting {
				"Removing saved login…"
			} else {
				"Credential queue unavailable; saved login may remain"
			};
		} else if !was_demo {
			self.credential_status = "Signed out of this account; its saved login is kept";
		}
		self.confirming_logout = false;
		self.end_intent = SessionEnd::Logout;
	}
	/// Confirms first when work would be lost, then ends the session for `intent`.
	fn request_session_end(&mut self, ctx: &egui::Context, intent: SessionEnd) {
		if self.state.has_unsent()
			|| self.messaging.has_edit()
			|| self.messaging.has_server_settings_changes()
			|| self.messaging.extensions.theme_editor_dirty()
			|| self.state.server_settings.pending
			|| self.state.server_admin.pending
			|| self.uploads.has_unsent()
		{
			self.end_intent = intent;
			self.confirming_logout = true;
		} else {
			self.finish_session_end(ctx, intent);
		}
	}
	fn finish_session_end(&mut self, ctx: &egui::Context, intent: SessionEnd) {
		self.end_session(ctx, intent);
		if let SessionEnd::Switch(account) = intent {
			self.begin_switch(account);
		}
	}
	/// Reads the saved token of an account already signed in on this device.
	fn begin_switch(&mut self, account: model::Id) {
		if self.fixture_only || self.state.demo {
			return;
		}
		let started = self.store.as_mut().is_some_and(|store| {
			store.load_account(self.state.generation, account, std::time::Instant::now())
		});
		self.switching = started.then_some(account);
		self.credential_status = if started {
			"Reading the saved login for that account…"
		} else {
			"Credential lookup unavailable; sign in with Discord again"
		};
	}
	/// Removes one saved account: its token, roster entry and cached data.
	fn forget_saved_account(&mut self, ctx: &egui::Context, account: model::Id) {
		if self
			.state
			.user
			.as_ref()
			.is_some_and(|user| user.id == account)
		{
			self.request_session_end(ctx, SessionEnd::Logout);
			return;
		}
		self.messaging.accounts.retain(|saved| saved.id != account);
		if let Some(store) = &self.store {
			let _ = store.send.try_send((
				self.state.generation,
				credentials::Request::NONE,
				credentials::Operation::ForgetAccount(account),
			));
		}
		if self.queue_cache_for(account, cache::Operation::Forget) {
			self.credential_status = "Saved account removed from this device";
		} else {
			self.cache_error = true;
			self.cache_status = "Could not queue local account data removal";
		}
	}
	/// Keeps the switcher entry for the signed-in account current, without retrying failures.
	fn sync_account_roster(&mut self) {
		if self.fixture_only || self.state.demo || self.roster_pending || self.roster_failed {
			return;
		}
		if self.state.auth != AuthState::Authenticated {
			return;
		}
		let Some(user) = &self.state.user else { return };
		let display = self
			.state
			.own_profile
			.data
			.as_ref()
			.and_then(|profile| profile.global_name.as_deref());
		// Re-record once per session so the switcher orders by last use, then only on change.
		let recorded = self.roster_generation == Some(self.state.generation);
		if recorded
			&& self.messaging.accounts.iter().any(|saved| {
				saved.id == user.id
					&& saved.name == user.name
					&& saved.avatar == user.avatar
					&& saved.discriminator == user.discriminator
					&& saved.display.as_deref() == display
			}) {
			return;
		}
		let account = model::SavedAccount {
			id: user.id,
			name: user.name.clone(),
			display: display.map(str::to_owned),
			avatar: user.avatar.clone(),
			discriminator: user.discriminator,
			has_token: false,
		};
		if !account.is_valid() {
			return;
		}
		self.roster_pending =
			self.queue_cache_for(model::Id(0), cache::Operation::SaveAccount(account));
		self.roster_failed = !self.roster_pending;
		if self.roster_pending {
			self.roster_generation = Some(self.state.generation);
		}
	}
	fn queue_cache(&mut self, operation: cache::Operation) -> bool {
		if let Some(user) = &self.state.user {
			if matches!(operation, cache::Operation::ClearHistory) {
				self.request_history_clear(user.id);
				return true;
			}
			self.queue_cache_for(user.id, operation)
		} else {
			false
		}
	}
	fn queue_cache_for(&mut self, account: model::Id, operation: cache::Operation) -> bool {
		if self.state.demo || self.fixture_only {
			return false;
		}
		if let Some(cache) = &self.cache {
			if matches!(
				operation,
				cache::Operation::LoadChannel { .. }
					| cache::Operation::SaveChannel { .. }
					| cache::Operation::SaveChanges { .. }
			) && !cache.history.allows(cache.history.epoch())
			{
				return false;
			}
			if cache.queue(self.state.generation, account, operation) {
				self.cache_pending += 1;
				if !self.cache_error {
					self.cache_status = "Saving local changes…";
				}
				return true;
			} else {
				self.cache_error = true;
				self.cache_status = "Local storage queue full; some changes are not saved";
			}
		}
		false
	}
	fn save_app_preferences(&mut self) {
		if self.fixture_only || self.state.demo {
			return;
		}
		self.app_settings.observe(&self.messaging);
		self.cache_pending += usize::from(
			self.app_settings
				.save(self.cache.as_ref(), self.state.generation),
		);
	}
	fn accept_font(&mut self, ctx: &egui::Context, result: &font_import::Selected) {
		self.messaging.custom_font.busy = false;
		match result {
			Ok(font) => {
				ui::fonts::apply_custom(ctx, font.as_ref());
				self.messaging.custom_font.name = font.as_ref().map(|font| font.name.clone());
				self.messaging.custom_font.status = "";
			}
			Err(error) => self.messaging.custom_font.status = error,
		}
	}
	fn save_font(&mut self, ctx: &egui::Context, font: Option<ui::fonts::CustomFont>) {
		if self.fixture_only || self.state.demo {
			self.accept_font(ctx, &Ok(font));
			self.messaging.custom_font.status = "Preview only; this font is not saved.";
		} else {
			self.messaging.custom_font.busy =
				self.queue_cache_for(model::Id(0), cache::Operation::SaveCustomFont(font));
			self.messaging.custom_font.status = if self.messaging.custom_font.busy {
				"Saving font…"
			} else {
				"Could not save the font. Try again."
			};
		}
	}
	fn sync_fonts(&mut self, ctx: &egui::Context) {
		if let Some(picker) = &self.font_picker {
			let result = match picker.try_recv() {
				Ok(result) => Some(result),
				Err(std::sync::mpsc::TryRecvError::Empty) => None,
				Err(std::sync::mpsc::TryRecvError::Disconnected) => {
					Some(Err("Font import interrupted. Try again."))
				}
			};
			if let Some(result) = result {
				self.font_picker = None;
				self.messaging.custom_font.busy = false;
				self.messaging.custom_font.status = "";
				match result {
					Ok(Some(font)) => self.save_font(ctx, Some(font)),
					Ok(None) => {}
					Err(error) => self.messaging.custom_font.status = error,
				}
			}
		}
		if let Some(action) = self.messaging.custom_font.request.take()
			&& !self.messaging.custom_font.busy
		{
			match action {
				ui::fonts::Action::Import => {
					self.font_picker =
						Some(font_import::choose(&self.runtime, ctx, self.window.clone()));
					self.messaging.custom_font.busy = true;
					self.messaging.custom_font.status = "Choosing font…";
				}
				ui::fonts::Action::Reset => self.save_font(ctx, None),
			}
		}
	}
	fn save_reading_preferences(&mut self, ctx: &egui::Context) {
		if self.fixture_only {
			return;
		}
		let now = std::time::Instant::now();
		// Finish changes made before entering preview; never persist preview controls.
		if !self.state.demo {
			self.reading
				.observe(self.messaging.reading_preferences, now);
			if std::mem::take(&mut self.messaging.reading_save_requested) {
				self.reading.request_save(now);
			}
		}
		if self.reading.ready(now) {
			let accepted = self.cache.as_ref().is_some_and(|cache| {
				cache.queue(
					self.state.generation,
					model::Id(0),
					cache::Operation::SaveReadingPreferences(self.reading.current),
				)
			});
			self.reading.queued(accepted);
			self.cache_pending += usize::from(accepted);
		}
		if let Some(delay) = self.reading.remaining(now) {
			ctx.request_repaint_after(delay);
		}
		self.messaging.reading_status = self.reading.status();
	}
	fn sync_tray(&mut self, ctx: &egui::Context) {
		let previous_status = self.messaging.tray_status;
		self.tray_setting.observe(self.messaging.minimize_to_tray);
		if self.tray_setting.dirty && !self.tray_setting.saving {
			self.tray_error = None;
			let accepted = !self.fixture_only
				&& !self.state.demo
				&& self.cache.as_ref().is_some_and(|cache| {
					cache.queue(
						self.state.generation,
						model::Id(0),
						cache::Operation::SaveMinimizeToTray(self.tray_setting.enabled),
					)
				});
			self.tray_setting.dirty = false;
			self.tray_setting.saving = accepted;
			self.tray_setting.failed = !accepted && !self.fixture_only && !self.state.demo;
			self.cache_pending += usize::from(accepted);
		}
		if !self.tray_setting.enabled {
			self.tray = None;
			self.tray_error = None;
		} else if self.tray_error.is_some() {
			self.tray = None;
		} else if self.tray.is_none() {
			let wake = ctx.clone();
			#[cfg(target_os = "linux")]
			let tray = {
				let _runtime = self.runtime.enter();
				platform::tray::Tray::new(
					move || wake.request_repaint(),
					self.tray_window.restorer(),
				)
			};
			#[cfg(not(target_os = "linux"))]
			let tray = platform::tray::Tray::new(self.window.clone(), move || wake.request_repaint());
			match tray {
				Ok(tray) => self.tray = Some(tray),
				Err(error) => self.tray_error = Some(error),
			}
		}
		if self.tray_window.hidden && !self.tray_available() {
			self.tray_window.show(ctx);
		}
		self.messaging.tray_status = self
			.tray_error
			.unwrap_or_else(|| self.tray_setting.status());
		if previous_status != self.messaging.tray_status {
			ctx.request_repaint();
		}
	}
	fn tray_available(&self) -> bool {
		if !self.tray_setting.enabled || self.tray_error.is_some() {
			return false;
		}
		#[cfg(target_os = "linux")]
		{
			self.tray
				.as_ref()
				.is_some_and(platform::tray::Tray::is_available)
		}
		#[cfg(not(target_os = "linux"))]
		{
			self.tray.is_some()
		}
	}
	fn presence_snapshot(&self) -> model::OwnPresence {
		model::OwnPresence {
			expires_at_ms: self.messaging.own_presence_expires,
			..self.messaging.own_presence.clone()
		}
	}
	fn adopt_remote_presence(&mut self, apply: bool) {
		let Some(connection) = &mut self.connection else {
			return;
		};
		if !connection.account_presence.has_changed().unwrap_or(false) {
			return;
		}
		let remote = connection.account_presence.borrow_and_update().clone();
		let Some(remote) = remote else {
			return;
		};
		if !apply {
			return;
		}
		self.messaging.adopt_account_presence(remote);
		self.presence_authoritative = true;
	}
	fn persist_account_presence(&mut self) {
		if !self.presence_authoritative || self.state.demo || self.fixture_only {
			return;
		}
		let Some(account) = self.state.user.as_ref().map(|user| user.id) else {
			return;
		};
		let snapshot = self.presence_snapshot();
		if !snapshot.valid() || self.presence_saved.as_ref() == Some(&(account, snapshot.clone())) {
			return;
		}
		if self.queue_cache(cache::Operation::SaveAccountPresence(snapshot.clone())) {
			self.account_presences.insert(account, snapshot.clone());
			self.presence_saved = Some((account, snapshot));
		}
	}
	fn flush_deferred_connect(&mut self, ctx: &egui::Context) {
		if self.presence_load_pending {
			return;
		}
		if let Some((secret, save)) = self.deferred_connect.take() {
			self.connect(secret, save, ctx);
		}
	}
	fn sync_own_presence(&mut self, ctx: &egui::Context) {
		self.expire_own_status(ctx);
		let changed = std::mem::take(&mut self.messaging.own_presence_changed);
		self.adopt_remote_presence(!changed);
		if changed {
			self.presence_authoritative = true;
		}
		let published = self.presence_snapshot();
		let previous_status = self.messaging.own_presence_status;
		self.messaging.own_presence_status = if !published.valid() {
			"Status must be at most 128 characters without line breaks or surrounding spaces."
		} else if self.state.demo || self.fixture_only {
			"Offline preview: not shared or saved."
		} else if let Some(connection) = &self.connection {
			let save_error = *connection.presence_error.borrow();
			let rejected = connection.own_presence.is_closed()
				|| (changed && connection.own_presence.send(published.clone()).is_err());
			if changed && !rejected {
				connection.presence_edits.send_replace(Some(published));
			}
			if rejected {
				"Could not update status: connection unavailable."
			} else if !self.state.gateway_connected {
				"Waiting for connection."
			} else {
				save_error.unwrap_or("")
			}
		} else {
			"Not connected; status is not shared."
		};
		self.persist_account_presence();
		if changed || previous_status != self.messaging.own_presence_status {
			ctx.request_repaint();
		}
	}
	/// A status with a "Clear after" deadline clears itself, then republishes like any edit.
	fn expire_own_status(&mut self, ctx: &egui::Context) {
		let Some(expires) = self.messaging.own_presence_expires else {
			return;
		};
		if self.messaging.own_presence.custom_status.is_empty() {
			self.messaging.own_presence_expires = None;
			return;
		}
		let now = std::time::SystemTime::now()
			.duration_since(std::time::UNIX_EPOCH)
			.unwrap_or_default()
			.as_millis()
			.min(u128::from(u64::MAX)) as u64;
		if now < expires {
			// Wake once at the deadline; egui otherwise sleeps through it on an idle window.
			ctx.request_repaint_after(std::time::Duration::from_millis(
				(expires - now).min(60_000),
			));
			return;
		}
		self.messaging.own_presence_expires = None;
		self.messaging.own_presence.expires_at_ms = None;
		self.messaging.own_presence.custom_status.clear();
		self.messaging.own_presence_changed = true;
		self.messaging
			.toasts
			.push(ui::design::Level::Info, "Custom status cleared");
	}
	fn sync_game_activity(&mut self, ctx: &egui::Context) {
		let previous_sharing = (
			self.messaging.discord_activity_sharing,
			self.messaging.discord_activity_sharing_busy,
			self.messaging.discord_activity_sharing_retry,
		);
		let previous = (
			self.messaging.own_game.clone(),
			self.messaging.game_activity_status,
		);
		#[cfg(feature = "demo")]
		if self.state.demo {
			let activity = self
				.messaging
				.share_game_activity
				.then(game_activity::demo_activity);
			self.messaging.own_game = activity.as_ref().map(model::RichActivity::summary);
			let changed = self.state.set_local_game_activity(activity);
			self.messaging.game_activity_status =
				"Offline preview: synthetic activity, never shared or saved.";
			if changed
				|| previous
					!= (
						self.messaging.own_game.clone(),
						self.messaging.game_activity_status,
					) {
				ctx.request_repaint();
			}
			return;
		}
		if self.fixture_only {
			return;
		}
		self.game_activity
			.observe(self.messaging.share_game_activity);
		if self.game_activity.dirty && !self.game_activity.saving {
			let accepted = self.cache.as_ref().is_some_and(|cache| {
				cache.queue(
					self.state.generation,
					model::Id(0),
					cache::Operation::SaveGameActivity(self.game_activity.enabled),
				)
			});
			self.game_activity.dirty = false;
			self.game_activity.saving = accepted;
			self.game_activity.failed = !accepted;
			self.cache_pending += usize::from(accepted);
		}
		self.messaging.own_game = None;
		self.messaging.discord_activity_sharing = None;
		self.messaging.discord_activity_sharing_busy = false;
		self.messaging.discord_activity_sharing_retry = false;
		let sharing_request = self.messaging.discord_activity_sharing_request.take();
		let mut own_activity = None;
		self.messaging.game_activity_status = self.game_activity.status();
		if let Some(connection) = &self.connection {
			connection.share_activity.send_if_modified(|enabled| {
				if *enabled == self.game_activity.enabled {
					return false;
				}
				*enabled = self.game_activity.enabled;
				true
			});
			// A local client may ask for an invite once; the dialog then waits for the user.
			if let Some((count, code)) = connection.rpc_invite.borrow().clone()
				&& self.rpc_invite_seen < count
			{
				self.rpc_invite_seen = count;
				self.messaging
					.open_rpc_invite(self.state.generation, code.clone());
			}
			if self.game_activity.enabled && self.state.gateway_connected {
				match &*connection.game_activity.borrow() {
					Ok(game) => {
						self.messaging.own_game = game.as_ref().map(model::RichActivity::summary);
						own_activity = game.clone();
						if game.is_some() && !self.game_activity.needs_attention() {
							use discord_gateway::ActivityObservation as Observation;
							self.messaging.game_activity_status = match *connection
								.activity_observation
								.borrow()
							{
								Observation::Unconfirmed => {
									"Local preview only. Waiting for Discord to confirm sharing."
								}
								Observation::ServerReceived => {
									"Discord received your game, but has not listed it publicly."
								}
								Observation::ServerListed => {
									"Discord lists your game. Server and friend privacy settings still apply."
								}
								Observation::ServerHidden => {
									self.messaging.discord_activity_sharing_retry = true;
									"Discord is hiding your game. Check Registered Games and Activity Sharing in Discord."
								}
								Observation::ServerMissing => {
									"Discord did not list your game publicly. Check its Registered Games and server sharing controls."
								}
							};
						}
					}
					Err(error) => self.messaging.game_activity_status = error,
				}
			}
			if self.game_activity.enabled && self.state.gateway_connected {
				match *connection.activity_sharing.borrow() {
					Ok(value) => {
						self.messaging.discord_activity_sharing = value;
						self.messaging.discord_activity_sharing_busy = value.is_none();
						if !self.game_activity.needs_attention() {
							match value {
								Some(false) => {
									self.messaging.game_activity_status =
										"Discord's account-wide activity sharing is off."
								}
								None => {
									self.messaging.game_activity_status =
										"Checking Discord's activity sharing setting..."
								}
								Some(true) => {}
							}
						}
					}
					Err(_) => {
						self.messaging.discord_activity_sharing_retry = true;
						if !self.game_activity.needs_attention() {
							self.messaging.game_activity_status =
								"Could not check or change Discord's activity sharing setting.";
						}
					}
				}
				if let Some(enable) = sharing_request {
					if connection.activity_sharing_request.try_send(enable).is_ok() {
						self.messaging.discord_activity_sharing_busy = true;
						self.messaging.game_activity_status =
							"Updating Discord's activity sharing setting...";
					} else {
						self.messaging.discord_activity_sharing_retry = true;
						self.messaging.game_activity_status =
							"Could not request the setting change. Try again.";
					}
				}
			}
		}
		// Keep the existing game preview; Spotify fills the activity card while no game is active.
		if own_activity.is_none() && self.state.gateway_connected {
			own_activity = self.connection.as_ref().and_then(|connection| {
				connection
					.spotify_activity
					.borrow()
					.as_ref()
					.map(|activity| activity.display())
			});
		}
		let changed = self.state.set_local_game_activity(own_activity);
		if changed
			|| previous_sharing
				!= (
					self.messaging.discord_activity_sharing,
					self.messaging.discord_activity_sharing_busy,
					self.messaging.discord_activity_sharing_retry,
				) || previous
			!= (
				self.messaging.own_game.clone(),
				self.messaging.game_activity_status,
			) {
			ctx.request_repaint();
		}
	}
	fn request_history_clear(&mut self, account: model::Id) {
		if self.state.demo || self.fixture_only {
			return;
		}
		let Some(cache) = &self.cache else {
			return;
		};
		cache.history.invalidate();
		cache.history.block();
		if !self.cache_clears.request(account) {
			cache.history.fail();
			self.cache_error = true;
			self.cache_status = "Cache cleanup backlog exceeded; history cache disabled until restart; deleted messages may remain on disk";
			return;
		}
		if !self.cache_error {
			self.cache_status =
				"Waiting to clear cached history; cached history temporarily disabled";
		}
		self.retry_history_clears();
	}
	fn retry_history_clears(&mut self) {
		let Some(cache) = &self.cache else {
			return;
		};
		for _ in 0..16 {
			let Some(account) = self.cache_clears.next() else {
				break;
			};
			if !cache.queue(
				self.state.generation,
				account,
				cache::Operation::ClearHistory,
			) {
				break;
			}
			self.cache_clears.queued(account);
			self.cache_pending += 1;
		}
	}
	fn delete_cached_messages(&mut self, event: &Event) {
		if self.state.demo || self.fixture_only {
			return;
		}
		let Some(account) = self.state.user.as_ref().map(|user| user.id) else {
			return;
		};
		let (channel, ids) = match event {
			Event::Delete { channel, id } => (*channel, vec![*id]),
			Event::DeleteBulk { channel, ids } if !ids.is_empty() && ids.len() <= 100 => {
				(*channel, ids.clone())
			}
			Event::DeleteBulk { ids, .. } if ids.len() > 100 => {
				self.request_history_clear(account);
				return;
			}
			_ => return,
		};
		self.delete_cached_ids(channel, ids);
	}
	fn delete_cached_ids(&mut self, channel: model::Id, ids: Vec<model::Id>) {
		if self.state.demo || self.fixture_only || ids.is_empty() {
			return;
		}
		let Some(account) = self.state.user.as_ref().map(|user| user.id) else {
			return;
		};
		let Some(cache) = &self.cache else {
			return;
		};
		if cache.delete_messages(self.state.generation, account, channel, ids) {
			self.cache_pending += 1;
		} else {
			self.request_history_clear(account);
		}
	}
	/// Dispatches one queued command to the demo or live transport.
	fn command(&mut self, command: Command) {
		if matches!(&command, Command::Interaction(client_core::interactions::Request {data:client_core::interactions::Data::Modal{components,..},..}) if interaction_uploads::has_files(components))
		{
			self.interaction_upload(command);
			return;
		}
		if let Command::ServerAdmin {
			guild,
			request,
			action,
		} = &command
			&& !self
				.state
				.server_admin_command_allowed(*guild, *request, action)
		{
			self.state.command_rejected(command);
			return;
		}
		if let Command::ServerSettings {
			guild,
			request,
			edit,
		} = &command
			&& !self
				.state
				.server_settings_command_allowed(*guild, *request, edit)
		{
			self.state.command_rejected(command);
			return;
		}
		if let Command::Send { channel, nonce, .. } = &command
			&& self
				.state
				.pending
				.iter()
				.any(|p| p.nonce == *nonce && !p.attachments.is_empty())
		{
			let (channel, nonce) = (*channel, nonce.clone());
			let available = !self.state.demo
				&& self.state.can_attach(channel)
				&& !self.fixture_only
				&& self.state.auth == AuthState::Authenticated
				&& self.state.gateway_connected
				&& self.state.freshness == model::Freshness::Fresh
				&& self.state.selected == Some(channel)
				&& self.connection.is_some();
			if available
				&& let Some(source) = self.uploads.take_source(self.state.generation, channel)
			{
				let (progress, receive) =
					tokio::sync::watch::channel(discord_api::upload::Status::Preparing);
				let (cancel, _) = tokio::sync::watch::channel(false);
				if self.uploads.begin_upload(receive, cancel.clone()).is_ok() {
					let request = uploads::UploadRequest {
						command,
						source,
						progress,
						cancel,
					};
					self.messaging.attachment = None;
					if let Err(error) = self.connection.as_ref().unwrap().uploads.try_send(request)
					{
						let request = error.into_inner();
						request
							.progress
							.send_replace(discord_api::upload::Status::Failed(
								"Upload queue full; reselect the file",
							));
						self.state.command_rejected(request.command);
					}
					return;
				}
			}
			self.state.apply(Envelope {
				generation: self.state.generation,
				event: Event::SendResult {
					nonce,
					result: Err(Failure::ProtocolAt(
						"File not sent; reconnect and reselect the attachment",
					)),
				},
			});
			return;
		}
		// A forum post with files takes the same staged-upload path as a message.
		if let Command::CreatePost {
			parent,
			attachments,
			request,
			..
		} = &command
			&& !attachments.is_empty()
		{
			let (parent, request) = (*parent, *request);
			let available = !self.state.demo
				&& self.state.can_attach_post(parent)
				&& !self.fixture_only
				&& self.state.auth == AuthState::Authenticated
				&& self.state.gateway_connected
				&& self.state.selected == Some(parent)
				&& self.connection.is_some();
			if available
				&& let Some(source) = self.uploads.take_source(self.state.generation, parent)
			{
				let (progress, receive) =
					tokio::sync::watch::channel(discord_api::upload::Status::Preparing);
				let (cancel, _) = tokio::sync::watch::channel(false);
				if self.uploads.begin_upload(receive, cancel.clone()).is_ok() {
					let request = uploads::UploadRequest {
						command,
						source,
						progress,
						cancel,
					};
					self.messaging.attachment = None;
					if let Err(error) = self.connection.as_ref().unwrap().uploads.try_send(request)
					{
						let request = error.into_inner();
						request
							.progress
							.send_replace(discord_api::upload::Status::Failed(
								"Upload queue full; reselect the file",
							));
						self.state.command_rejected(request.command);
					}
					return;
				}
			}
			self.state.apply_post(
				parent,
				request,
				Err(Failure::ProtocolAt(
					"Post not created; reconnect and reselect the attachment",
				)),
			);
			return;
		}
		if let Command::History {
			channel,
			before: None,
			after: None,
			..
		} = &command
			&& !self.fixture_only
			&& self.state.selected == Some(*channel)
			&& self.state.can_call(*channel)
			&& self
				.state
				.channels
				.iter()
				.any(|c| c.id == *channel && c.guild.is_none())
		{
			self.command(Command::Voice(client_core::voice::Command::Sync {
				channel: *channel,
			}));
		}
		if let Command::Voice(control) = &command {
			if self.state.demo || self.fixture_only {
				self.state.status = "Voice calls are unavailable in the offline preview";
				return;
			}
			if let client_core::voice::Command::Join {
				channel,
				request,
				ring,
				..
			} = control
			{
				let result = self.voice.begin(&self.state, *ring);
				if let Err(message) = result {
					self.state.apply_voice(client_core::voice::Event::Failed {
						channel: *channel,
						request: *request,
						message,
					});
					return;
				}
			}
			if matches!(
				control,
				client_core::voice::Command::SetCamera { enabled: false, .. }
			) {
				self.voice.stop_camera();
				self.messaging.voice_camera_preview = None;
			}
			if matches!(control, client_core::voice::Command::Leave { .. }) {
				self.voice.stop();
				self.messaging.camera_test_requested = false;
				self.messaging.camera_test_texture = None;
			}
		}
		if let Command::History {
			channel,
			before: None,
			after: None,
			request,
		} = &command
			&& wants_cached_history(&self.state, *channel, *request)
		{
			self.queue_cache(cache::Operation::LoadChannel {
				channel: *channel,
				request: *request,
			});
		}
		#[cfg(feature = "demo")]
		if self.state.demo {
			let event = match command {
				Command::ApplicationCommands {
					channel,
					guild,
					request,
				} => Event::ApplicationCommands {
					channel,
					request,
					result: Ok(slash_demo::catalog(guild)),
				},
				Command::Interaction(request) => {
					if matches!(
						&request.data,
						client_core::interactions::Data::ApplicationCommand { .. }
					) {
						slash_demo::respond(&mut self.state, request);
						return;
					}
					if std::env::args().any(|arg| arg == "--demo-components") {
						self.state.apply(Envelope {
							generation: self.state.generation,
							event: components_demo::respond(request),
						});
						return;
					}
					Event::Interaction(client_core::interactions::Event::Submitted {
						nonce: request.nonce,
						result: Err(Failure::ProtocolAt(
							"Offline preview does not contact applications",
						)),
					})
				}
				Command::MemberSearch(request) => {
					let query = request.query.to_lowercase();
					let rows = demo_members(Some(request.guild), request.channel, request.nonce)
						.slots
						.into_iter()
						.flatten()
						.filter_map(|slot| match slot {
							model::MemberSlot::Person(member) => Some(member),
							_ => None,
						})
						.filter(|member| {
							member.user.name.to_lowercase().contains(&query)
								|| member
									.nick
									.as_ref()
									.is_some_and(|name| name.to_lowercase().contains(&query))
								|| member.user.id.to_string() == query
						})
						.collect();
					Event::MemberSearch {
						request,
						result: Ok(rows),
					}
				}

				// Demo preference changes are applied synchronously by client-core.
				Command::MessagingPermissions { .. } => return,
				Command::ChannelAction {
					guild,
					channel,
					request,
					action,
				} => channel_demo::execute(
					&self.state,
					guild,
					channel,
					request,
					action,
					&mut self.synthetic_id,
				),
				Command::ServerAdmin {
					guild,
					request,
					action,
				} => server_settings_demo::execute_admin(&self.state, guild, request, *action),
				Command::ServerSettings {
					guild,
					request,
					edit,
				} => server_settings_demo::execute(&self.state, guild, request, edit),
				Command::GuildFolders(settings) => Event::GuildFolders(Ok(settings
					.map(|(_, settings)| settings)
					.unwrap_or_default())),
				Command::SendServerInvite {
					guild,
					user,
					code,
					nonce,
					request,
				} => {
					let channel = dm_demo::channel(&self.state, user).unwrap();
					self.synthetic_id += 1;
					let mut message = test_support::message(self.synthetic_id, channel.id);
					message.author = self.state.user.clone().unwrap();
					message.content = format!("https://discord.gg/{code}");
					message.nonce = Some(nonce);
					Event::ServerAction(client_core::server_actions::Event::InviteSent {
						guild,
						user,
						request,
						result: Ok(Box::new((channel, message))),
					})
				}
				Command::ServerAction { action, request } => {
					server_settings_demo::execute_action(&mut self.state, action, request)
				}
				Command::UserAction {
					action: client_core::user_actions::Action::OpenDm(user),
					request,
					..
				} => Event::UserAction(client_core::user_actions::Event::DmOpened {
					user,
					request,
					result: dm_demo::channel(&self.state, user).map(Box::new),
				}),
				Command::UserAction {
					action: client_core::user_actions::Action::LoadNote(user),
					request,
					..
				} => Event::UserAction(client_core::user_actions::Event::NoteLoaded {
					user,
					request,
					result: Ok(self.state.user_note(user).unwrap_or("").to_owned()),
				}),
				Command::UserAction {
					action, request, ..
				} => Event::UserAction(client_core::user_actions::Event::Written {
					action,
					request,
					result: Ok(()),
				}),
				Command::GroupAction { action, request } => {
					use client_core::group_actions::{Action, Event as GroupEvent};
					use model::Patch;
					let channel = action.channel();
					let patch = match action {
						Action::Leave(_) => None,
						Action::Edit { name, icon, .. } => Some(model::ChannelPatch {
							id: channel,
							name: name.map_or(Patch::Absent, Patch::Value),
							icon: match icon {
								Patch::Absent => self
									.state
									.channel(channel)
									.and_then(|c| c.icon.clone())
									.map_or(Patch::Null, Patch::Value),
								Patch::Null => Patch::Null,
								Patch::Value(_) => {
									Patch::Value("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into())
								}
							},
							last_message: Patch::Absent,
							parent_id: Patch::Absent,
							position: Patch::Absent,
							kind: Patch::Absent,
							message_count: Patch::Absent,
							tags: Patch::Absent,
						}),
					};
					Event::GroupAction(GroupEvent::Written {
						channel,
						request,
						result: Ok(patch),
					})
				}
				Command::MarkRead {
					channel,
					message,
					request,
					..
				} => Event::ReadState(client_core::read_state::Event::Result {
					channel,
					message,
					request,
					result: Ok(()),
				}),
				Command::MarkGuildRead { guild, request } => {
					Event::ReadState(client_core::read_state::Event::GuildAck {
						guild,
						request,
						result: Ok(()),
					})
				}
				Command::Reactions(command) => {
					use client_core::reactions::{Command as R, Event as E};
					Event::Reactions(match command {
						R::Read {
							channel,
							message,
							request,
						} => E::Read {
							channel,
							message,
							request,
							result: Ok(vec![]),
						},
						R::Set {
							channel,
							message,
							emoji,
							add,
							request,
						} => {
							let mut reactions = self
								.state
								.timeline
								.get(message)
								.and_then(|m| m.reactions.clone())
								.unwrap_or_default();
							if let Some(r) = reactions.iter_mut().find(|r| r.emoji.same(&emoji)) {
								if r.me != add {
									r.count = if add {
										r.count + 1
									} else {
										r.count.saturating_sub(1)
									};
									r.me = add;
								}
							} else if add {
								reactions.push(model::Reaction {
									emoji,
									count: 1,
									me: true,
									me_burst: false,
								});
							}
							reactions.retain(|r| r.count > 0);
							self.state.reactions.reset();
							let _ = self.state.timeline.set_reactions(message, Some(reactions));
							// The fixture has no service readback; it updates synthetic RAM only.
							E::Written {
								channel,
								message,
								request,
								result: Ok(()),
							}
						}
						R::Users {
							channel,
							message,
							emoji,
							request,
							..
						} => {
							let users = self
								.state
								.timeline
								.get(message)
								.map(|message| vec![message.author.clone()])
								.unwrap_or_default();
							E::Users {
								channel,
								message,
								emoji,
								request,
								result: Ok(users),
							}
						}
					})
				}
				Command::Voice(_) | Command::CancelProfile | Command::CancelSearch => return,
				Command::ThreadStarter {
					thread,
					parent,
					request,
				} => {
					// The fixture thread hangs off a synthetic parent message with its own id.
					let mut message = test_support::message(thread.0, parent);
					message.content =
						"This synthetic message started the thread; replies continue below.".into();
					Event::ThreadStarter {
						thread,
						request,
						result: Ok(message),
					}
				}
				Command::CreatePost {
					parent,
					guild,
					title,
					tags,
					request,
					..
				} => {
					self.synthetic_id += 1;
					Event::PostCreated {
						parent,
						request,
						result: Ok(model::Channel {
							id: model::Id(self.synthetic_id),
							guild: Some(guild),
							parent_id: Some(parent),
							position: 0,
							name: title,
							icon: None,
							kind: 11,
							recipients: vec![],
							last_message: None,
							member_list_id: None,
							tags: (!tags.is_empty()).then(|| {
								Box::new(model::forum::Tags {
									applied: tags,
									..Default::default()
								})
							}),
							message_count: Some(0),
						}),
					}
				}
				Command::Archives {
					parent,
					guild,
					kind,
					before,
					request,
				} => {
					use model::archives::{Cursor, Kind, Page};
					let offset = parent.0.saturating_mul(10_000).saturating_add(match kind {
						Kind::Public => 0,
						Kind::Private => 1_000,
						Kind::JoinedPrivate => 2_000,
					});
					let ids = (if before.is_none() {
						[900, 850, 800]
					} else {
						[700, 650, 600]
					})
					.map(|id| offset.saturating_add(id));
					let public_kind = if self
						.state
						.channels
						.iter()
						.any(|c| c.id == parent && c.kind == 5)
					{
						10
					} else {
						11
					};
					let threads = ids
						.into_iter()
						.map(|id| model::Channel {
							id: model::Id(id),
							guild: Some(guild),
							parent_id: Some(parent),
							position: 0,
							name: format!("Synthetic archived thread {id}"),
							icon: None,
							kind: if kind == Kind::Public {
								public_kind
							} else {
								12
							},
							recipients: vec![],
							last_message: None,
							member_list_id: None,
							tags: None,
							message_count: None,
						})
						.collect();
					Event::Archives {
						parent,
						request,
						result: Ok(Page {
							threads,
							next: before.is_none().then_some(if kind == Kind::JoinedPrivate {
								Cursor::Id(model::Id(ids[2]))
							} else {
								Cursor::Time(1_788_998_400_000_000_000)
							}),
						}),
					}
				}
				Command::Pins {
					channel,
					before,
					request,
				} => {
					// Explicit synthetic pins, independent of message creation order.
					let hits = if before.is_none() {
						[480, 499, 470]
					} else {
						[420, 455, 430]
					}
					.into_iter()
					.map(|id| {
						let message = test_support::message(id, channel);
						model::SearchHit {
							id: message.id,
							channel,
							excerpt: format!(
								"Synthetic pinned message: {}",
								message.content.chars().take(200).collect::<String>()
							),
							author: message.author,
							attachments: message.attachments,
							embeds: message.embeds,
						}
					})
					.collect();
					Event::Search {
						channel,
						request,
						result: Ok(client_core::search::Outcome::Pins(model::SearchPage {
							hits,
							total: 0,
							partial: before.is_none(),
							pin_cursor: before.is_none().then_some(1_788_998_400_000_000_000),
						})),
					}
				}
				Command::Search {
					channel,
					query,
					before,
					request,
					..
				} => {
					let mut hits = Vec::new();
					let mut total = 0;
					let (content, filters) = match model::search_terms(&query) {
						Ok(terms) => terms,
						Err(_) => return,
					};
					for id in (1..=500)
						.rev()
						.filter(|id| before.is_none_or(|b| *id < b.0))
					{
						let message = test_support::message(id, channel);
						if message
							.content
							.to_lowercase()
							.contains(&content.to_lowercase())
							&& filters.iter().all(|(group, _)| {
								filters.iter().filter(|(key, _)| key == group).any(
									|(key, value)| match *key {
										"author_id" => message.author.id.to_string() == *value,
										"mentions" => message
											.mentions
											.iter()
											.any(|user| user.id.to_string() == *value),
										"min_id" => {
											value.parse::<u64>().is_ok_and(|min| message.id.0 > min)
										}
										"max_id" => {
											value.parse::<u64>().is_ok_and(|max| message.id.0 < max)
										}
										"pinned" => {
											self.state.is_pinned(channel, message.id)
												== (value == "true")
										}
										"author_type" => match value.as_str() {
											"webhook" => message.author.webhook,
											"user" => !message.author.webhook,
											_ => false, // The offline fixture contains no bot authors.
										},
										"has" => match value.as_str() {
											"link" => {
												message.content.contains("https://")
													|| message.content.contains("http://")
											}
											"embed" => !message.embeds.is_empty(),
											"file" => !message.attachments.is_empty(),
											"image" => message.attachments.iter().any(|a| {
												a.content_type
													.as_deref()
													.is_some_and(|mime| mime.starts_with("image/"))
											}),
											"video" => {
												message.attachments.iter().any(|a| a.is_video())
											}
											"sound" => {
												message.attachments.iter().any(|a| a.is_audio())
											}
											_ => false,
										},
										_ => false,
									},
								)
							}) {
							total += 1;
							if hits.len() < model::SEARCH_PAGE_SIZE {
								hits.push(model::SearchHit {
									id: message.id,
									channel,
									author: message.author,
									excerpt: message.content.clone(),
									attachments: message.attachments,
									embeds: message.embeds,
								});
							}
						}
					}
					Event::Search {
						channel,
						request,
						result: Ok(client_core::search::Outcome::Page(model::SearchPage {
							hits,
							total,
							partial: false,
							pin_cursor: None,
						})),
					}
				}
				Command::Gifs { query, request } => Event::Gifs {
					request,
					result: Ok(test_support::gif_page(query.as_deref())),
				},
				Command::CancelGifs => return,
				Command::CreateGuild { sequence, .. } => Event::GuildCreated {
					sequence,
					result: Err(Failure::ProtocolAt("Server creation unavailable offline")),
				},
				Command::JoinInvite { request, .. } => Event::JoinInvite {
					request,
					result: Err(Failure::ProtocolAt("Server joining unavailable offline")),
				},
				Command::Invite { code } => Event::Invite {
					code,
					result: Err(Failure::Protocol),
				},
				Command::Profile {
					user,
					guild,
					request,
				} => Event::Profile {
					user,
					guild,
					request,
					result: Err(Failure::Protocol),
				},
				Command::EditProfile {
					user,
					request,
					changes,
				} => {
					let result = self
						.state
						.user
						.as_ref()
						.filter(|own| own.id == user)
						.map(|own| {
							let mut profile = self
								.state
								.own_profile
								.data
								.clone()
								.unwrap_or_else(|| ui::synthetic_own_profile(own));
							if let Some(changes) = changes {
								if !changes.valid() {
									return Err(Failure::Capacity);
								}
								if let Some(name) = changes.global_name {
									profile.user.name =
										name.clone().unwrap_or_else(|| profile.username.clone());
									profile.global_name = name;
								}
								if let Some(bio) = changes.bio {
									profile.bio = bio;
								}
								if let Some(pronouns) = changes.pronouns {
									profile.pronouns = pronouns;
								}
								if let Some(color) = changes.accent_color {
									profile.accent_color = color;
								}
								if let Some(avatar) = changes.avatar {
									// Synthetic hash: the fixture never uploads or fetches images.
									profile.user.avatar = avatar
										.map(|_| "0123456789abcdef0123456789abcdef".to_owned());
								}
							}
							Ok(Box::new(profile))
						})
						.unwrap_or(Err(Failure::Protocol));
					Event::ProfileEdited {
						user,
						request,
						result,
					}
				}
				Command::Members {
					guild,
					channel,
					request,
					..
				} => {
					let Some(channel) = channel else {
						return;
					};
					Event::Members(demo_members(guild, channel, request))
				}
				Command::ForumPosts { .. } | Command::ForumSummaries { .. } => return,
				Command::History { before, after, .. } => {
					test_support::load_page_with_cursors(&mut self.state, before, after);
					return;
				}
				Command::StickerPacks => Event::StickerPacks(Ok(self.state.stickers.packs.clone())),
				Command::Sticker(id) => Event::Sticker {
					id,
					result: self
						.state
						.guilds
						.iter()
						.filter_map(|g| g.stickers.as_ref())
						.flatten()
						.chain(
							self.state
								.stickers
								.packs
								.iter()
								.flat_map(|p| p.stickers.iter()),
						)
						.find(|s| s.id == id)
						.cloned()
						.ok_or(Failure::Protocol),
				},
				Command::Forward {
					message,
					channel,
					nonce,
					..
				} => {
					self.synthetic_id += 1;
					let result = self
						.state
						.timeline
						.get(message)
						.cloned()
						.ok_or(Failure::Protocol)
						.map(|mut m| {
							m.id = model::Id(self.synthetic_id);
							m.channel = channel;
							m.author = self.state.user.clone().unwrap();
							m.author_nick = None;
							m.author_roles.clear();
							m.nonce = Some(nonce.clone());
							m.reply_to = None;
							m.reply_deleted = false;
							m.kind = 0;
							m.forwarded = true;
							m.reactions = None;
							m
						});
					Event::SendResult { nonce, result }
				}
				Command::Send {
					sticker,
					channel,
					content,
					nonce,
					reply,
				} => {
					self.synthetic_id += 1;
					let mut message = test_support::message(self.synthetic_id, channel);
					message.author = self.state.user.clone().unwrap();
					message.content = content;
					message.sticker_items = sticker
						.and_then(|id| {
							self.state
								.pending
								.iter()
								.find_map(|p| p.sticker.as_ref().filter(|s| s.id == id).cloned())
						})
						.into_iter()
						.collect();
					message.nonce = Some(nonce.clone());
					message.reply_to = reply.map(client_core::Reply::target);
					Event::SendResult {
						nonce,
						result: Ok(message),
					}
				}
				Command::Edit {
					request,
					channel,
					message,
					content,
				} => {
					let result = self
						.state
						.timeline
						.get(message)
						.cloned()
						.ok_or(Failure::Protocol)
						.map(|mut updated| {
							updated.content = content;
							updated.edited = true;
							updated.edited_at =
								Some(updated.edited_at.unwrap_or(0).saturating_add(1));
							updated
						});
					Event::Edited {
						request,
						channel,
						message,
						result,
					}
				}
				Command::Delete { channel, message } => Event::Delete {
					channel,
					id: message,
				},
				Command::Pin {
					request,
					channel,
					message,
					pinned,
				} => Event::Pinned {
					request,
					channel,
					message,
					pinned,
					result: Ok(()),
				},
			};
			self.state.apply(Envelope {
				generation: self.state.generation,
				event,
			});
			self.state.status = "Offline fixture · action affected synthetic RAM only";
			return;
		}
		if let Some(connection) = &self.connection {
			if let Err(error) = connection.commands.try_send(command) {
				self.state.command_rejected(error.into_inner());
			}
		} else {
			self.state.command_rejected(command);
		}
	}
	/// Boot stage while a saved login is being restored, so launch shows progress
	/// instead of a welcome card the user cannot act on yet.
	fn restoring(&self) -> Option<&'static str> {
		// Fixture-only preview of the restore screen, e.g. `--demo --demo-restoring`.
		#[cfg(feature = "demo")]
		if self.fixture_only && std::env::args().any(|arg| arg == "--demo-restoring") {
			return Some("Checking your saved login");
		}
		if self.fixture_only
			|| self.state.demo
			|| self.login.is_some()
			|| self.forgetting
			|| matches!(
				self.state.auth,
				AuthState::Failed | AuthState::Expired | AuthState::Challenged
			) {
			return None;
		}
		if self
			.store
			.as_ref()
			.is_some_and(|store| store.remaining(std::time::Instant::now()).is_some())
		{
			return Some("Checking your saved login");
		}
		self.connection.is_some().then_some("Connecting to Discord")
	}
	/// Restore screen for returning accounts: no sign-in controls, just the stage,
	/// an indeterminate bar and a way out to the welcome screen.
	fn restoring_screen(&mut self, ui: &mut egui::Ui, stage: &'static str) {
		let p = ui::design::palette(ui);
		egui::CentralPanel::default()
			.frame(egui::Frame::NONE.fill(ui::design::window_palette(ui).canvas))
			.show(ui, |ui| {
				accent_glow(ui);
				ui::design::window_drag(ui, ui.max_rect());
				egui::Frame::NONE
					.inner_margin(egui::Margin {
						left: (16.0 + self.traffic_light_inset()) as i8,
						right: if ui::design::WINDOW_CONTROLS_WIDTH > 0.0 {
							0
						} else {
							16
						},
						top: if ui::design::WINDOW_CONTROLS_WIDTH > 0.0 {
							0
						} else {
							16
						},
						bottom: 0,
					})
					.show(ui, |ui| {
						ui.horizontal(|ui| {
							ui.label(ui::design::semibold(ui, "Serein", 16.0).color(p.muted));
							ui.with_layout(
								egui::Layout::right_to_left(egui::Align::Center),
								|ui| {
									if !self.messaging.hide_title_bar {
										ui::design::window_controls(ui);
									}
								},
							);
						});
					});
				let time = ui.input(|i| i.time) as f32;
				ui.ctx().request_repaint();
				ui.vertical_centered(|ui| {
					ui.add_space((ui.available_height() * 0.5 - 150.0).max(12.0));
					// App mark with a slow breathing halo; the only motion besides the bar.
					let (rect, _) =
						ui.allocate_exact_size(egui::vec2(72.0, 72.0), egui::Sense::hover());
					let mark = egui::Rect::from_center_size(rect.center(), egui::Vec2::splat(56.0));
					let pulse = 0.5 + 0.5 * (time * 1.6).sin();
					ui.painter().rect_filled(
						mark.expand(4.0 + 4.0 * pulse),
						18,
						p.accent.gamma_multiply(0.10 + 0.10 * pulse),
					);
					ui.painter().rect_filled(mark, 14, p.accent);
					ui::icons::paint(
						ui.painter(),
						ui::icons::Icon::Serein,
						mark.shrink(13.0),
						p.accent_text,
					);
					ui.add_space(18.0);
					ui.label(ui::design::semibold(ui, "Welcome back", 24.0).color(p.text_strong));
					ui.add_space(6.0);
					ui.label(
						egui::RichText::new(format!("{stage}…"))
							.size(15.0)
							.color(p.muted),
					);
					ui.add_space(22.0);
					// Indeterminate track: progress is unknown, so a sweeping segment.
					let width = ui.available_width().min(260.0);
					let (track, _) =
						ui.allocate_exact_size(egui::vec2(width, 4.0), egui::Sense::hover());
					ui.painter().rect_filled(track, 2, p.raised);
					let span = track.width() * 0.35;
					let travel = (track.width() + span) * ((time * 0.5).fract());
					let left = (track.left() + travel - span).max(track.left());
					let right = (track.left() + travel).min(track.right());
					if right > left {
						ui.painter().rect_filled(
							egui::Rect::from_min_max(
								egui::pos2(left, track.top()),
								egui::pos2(right, track.bottom()),
							),
							2,
							p.accent,
						);
					}
					ui.add_space(26.0);
					ui.allocate_ui_with_layout(
						egui::vec2(220.0, 0.0),
						egui::Layout::top_down(egui::Align::Center),
						|ui| {
							if ui::design::secondary_button(ui, "Use a different account").clicked()
							{
								if let Some(store) = &mut self.store {
									store.cancel_load();
								}
								self.connection = None;
								self.pending_save = None;
								self.pending_account_save = None;
								self.state.auth = AuthState::Unauthenticated;
								self.state.status = "Disconnected";
								self.credential_status = "Saved-login restore cancelled";
							}
						},
					);
				});
			});
	}
	fn traffic_light_inset(&self) -> f32 {
		if self.messaging.hide_title_bar {
			0.0
		} else {
			ui::design::TRAFFIC_LIGHT_INSET
		}
	}
	fn sign_in_screen(&mut self, ui: &mut egui::Ui) {
		let p = ui::design::palette(ui);
		egui::CentralPanel::default()
			.frame(egui::Frame::NONE.fill(ui::design::window_palette(ui).canvas))
			.show(ui, |ui| {
				accent_glow(ui);
				egui::Panel::top("sign-in-header")
					.exact_size(SIGN_IN_HEADER_HEIGHT)
					.show_separator_line(false)
					.frame(egui::Frame::NONE)
					.show(ui, |ui| {
						// Drag first: later widgets win hit testing, so the header buttons stay clickable.
						ui::design::window_drag(ui, ui.max_rect());
						ui.horizontal_centered(|ui| {
							ui.add_space(16.0 + self.traffic_light_inset());
							// Wordmark lockup: app mark, name, then a quiet outlined stage pill.
							let (mark, _) = ui
								.allocate_exact_size(egui::vec2(22.0, 22.0), egui::Sense::hover());
							ui.painter().rect_filled(mark, 6, p.accent);
							ui::icons::paint(
								ui.painter(),
								ui::icons::Icon::Serein,
								mark.shrink(5.0),
								p.accent_text,
							);
							ui.add_space(8.0);
							ui.label(ui::design::semibold(ui, "Serein", 16.0).color(p.text_strong));
							ui.add_space(8.0);
							// Painted rather than framed: the pill must hug the text, not the row height.
							let stage = ui.painter().layout_no_wrap(
								"Early preview".to_owned(),
								egui::FontId::new(10.5, ui::design::medium_family(ui.ctx())),
								p.muted,
							);
							let (pill, _) = ui.allocate_exact_size(
								egui::vec2(stage.size().x + 16.0, 19.0),
								egui::Sense::hover(),
							);
							ui.painter().rect_stroke(
								pill,
								9,
								egui::Stroke::new(1.0, p.border),
								egui::StrokeKind::Inside,
							);
							ui.painter()
								.galley(pill.center() - stage.size() * 0.5, stage, p.muted);
							ui.with_layout(
								egui::Layout::right_to_left(egui::Align::Center),
								|ui| {
									if ui::design::WINDOW_CONTROLS_WIDTH > 0.0
										&& !self.messaging.hide_title_bar
									{
										ui::design::window_controls(ui);
									} else {
										ui.add_space(24.0);
									}
									ui.add_space(12.0);
									// These popups hold settings controls, so a click inside
									// must not dismiss them the way a menu command would.
									let sticky = || {
										egui::containers::menu::MenuConfig::new().close_behavior(
											egui::PopupCloseBehavior::CloseOnClickOutside,
										)
									};
									// Quiet header actions: text only until hovered, like the rest of the chrome.
									let quiet =
										|ui: &egui::Ui, text: &str, color: egui::Color32| {
											egui::Button::new(
												ui::design::medium(ui, text, 13.0).color(color),
											)
											.frame_when_inactive(false)
											.corner_radius(8)
										};
									egui::containers::menu::MenuButton::from_button(quiet(
										ui,
										"Appearance",
										p.muted,
									))
									.config(sticky())
									.ui(ui, |ui| self.messaging.appearance_menu(ui));
									ui.add_space(8.0);
									let updates = &self.messaging.updates;
									let (label, color) = if updates.ready {
										("Restart to update", p.link)
									} else if updates.busy {
										("Updating…", p.muted)
									} else if updates.available {
										("Update available", p.link)
									} else {
										("Updates", p.muted)
									};
									let demo = self.fixture_only || self.state.demo;
									egui::containers::menu::MenuButton::from_button(quiet(
										ui, label, color,
									))
									.config(sticky())
									.ui(ui, |ui| self.messaging.updates_menu(ui, demo));
								},
							);
						});
					});
				egui::ScrollArea::vertical()
					.id_salt("sign-in-scroll")
					.show(ui, |ui| {
						let card = if self.sign_in_height > 0.0 {
							self.sign_in_height + 72.0
						} else {
							560.0
						};
						ui.add_space(((ui.available_height() - card) * 0.4).max(16.0));
						ui.vertical_centered(|ui| {
							ui.allocate_ui_with_layout(
								egui::vec2(ui.available_width().min(460.0), 0.0),
								egui::Layout::top_down(egui::Align::Min),
								|ui| {
									self.sign_in_card(ui);
									#[cfg(feature = "demo")]
									if self.fixture_only {
										self.sign_in_preview(ui);
									}
									// Centring needs the height this actually took; it is
									// stable between frames.
									self.sign_in_height = ui.min_rect().height();
								},
							);
							ui.add_space(18.0);
							ui.label(
								egui::RichText::new(
									"Independent and open source. Not affiliated with Discord.",
								)
								.size(12.0)
								.color(p.muted),
							);
							ui.add_space(24.0);
						});
					});
			});
	}
	/// Sign-in card: saved accounts first for returning owners, one clear primary action,
	/// explicit consent, and everything else folded away until asked for.
	fn sign_in_card(&mut self, ui: &mut egui::Ui) {
		let ctx = ui.ctx().clone();
		let p = ui::design::palette(ui);
		let shadow = egui::epaint::Shadow {
			offset: [0, 16],
			blur: 40,
			spread: 0,
			color: egui::Color32::from_black_alpha(if ui.visuals().dark_mode { 110 } else { 34 }),
		};
		egui::Frame::NONE
			.fill(p.surface)
			.stroke(egui::Stroke::new(1.0, p.border))
			.shadow(shadow)
			.corner_radius(16)
			.inner_margin(egui::Margin {
				left: 28,
				right: 28,
				top: 24,
				bottom: 14,
			})
			.show(ui, |ui| {
				let returning = !self.messaging.accounts.is_empty();
				let waiting = self.state.auth == AuthState::Authenticating;
				let idle = !waiting && !self.forgetting;
				// A new sign-in needs explicit authorization; a saved account was already
				// authorized once and restores unattended on launch, so it only waits for idle.
				// Fixture builds render the enabled state for captures; the actions stay inert.
				let ready = self.authorized && idle;
				self.sign_in_header(ui, returning);
				ui.add_space(16.0);
				if returning {
					self.sign_in_accounts(ui, idle);
					ui.add_space(10.0);
				}
				let label = if waiting {
					"Waiting for Discord…"
				} else if returning {
					"Use another account"
				} else {
					"Continue with Discord"
				};
				let button = ui
					.add_enabled_ui(ready, |ui| {
						if returning {
							ui::design::secondary_icon_button(ui, ui::icons::Icon::Plus, label)
						} else {
							ui::design::primary_icon_button(ui, ui::icons::Icon::Serein, label)
						}
					})
					.inner;
				if button.clicked() && !self.fixture_only {
					if let Some(store) = &mut self.store {
						store.cancel_load();
					}
					self.credential_status = "Sign in through Discord; saved-login lookup stopped";
					let wake = ctx.clone();
					match platform::LoginView::open(self.window.clone(), move || {
						wake.request_repaint()
					}) {
						Ok(login) => {
							self.login = Some(login);
							self.state.auth = AuthState::Authenticating;
							self.state.status = "Waiting for Discord login";
						}
						Err(_) => {
							self.state.auth = AuthState::Failed;
							self.state.status =
								"Platform login webview unavailable; see platform-support.md";
						}
					}
				}
				ui.add_space(10.0);
				self.sign_in_consent(ui);
				self.sign_in_status(ui);
				ui.add_space(12.0);
				let (line, _) = ui.allocate_exact_size(
					egui::vec2(ui.available_width(), 1.0),
					egui::Sense::hover(),
				);
				ui.painter().rect_filled(line, 0, p.border);
				ui.add_space(8.0);
				self.sign_in_disclosures(ui, &ctx);
			});
	}
	/// App mark over a welcome line that adapts to whether this device already knows an account.
	fn sign_in_header(&self, ui: &mut egui::Ui, returning: bool) {
		let p = ui::design::palette(ui);
		ui.vertical_centered(|ui| {
			let (mark, _) = ui.allocate_exact_size(egui::vec2(44.0, 44.0), egui::Sense::hover());
			ui.painter()
				.rect_filled(mark.expand(6.0), 17, p.accent.gamma_multiply(0.16));
			ui.painter().rect_filled(mark, 13, p.accent);
			ui::icons::paint(
				ui.painter(),
				ui::icons::Icon::Serein,
				mark.shrink(11.0),
				p.accent_text,
			);
			ui.add_space(12.0);
			ui.label(
				ui::design::semibold(
					ui,
					if returning {
						"Welcome back"
					} else {
						"Welcome to Serein"
					},
					22.0,
				)
				.color(p.text_strong),
			);
			ui.add_space(5.0);
			ui.add(
				egui::Label::new(
					egui::RichText::new(if returning {
						"Continue with a saved account, or sign in with another one."
					} else {
						"Sign in with your Discord account to get started."
					})
					.size(14.0)
					.color(p.muted),
				)
				.wrap(),
			);
		});
	}
	/// Accounts already signed in on this device: one tap restores their saved login.
	fn sign_in_accounts(&mut self, ui: &mut egui::Ui, enabled: bool) {
		let p = ui::design::palette(ui);
		ui.label(ui::design::eyebrow(ui, "Saved accounts", p.muted));
		ui.add_space(6.0);
		let saved: Vec<(model::Id, String, String)> = self
			.messaging
			.accounts
			.iter()
			.map(|account| {
				(
					account.id,
					account.label().to_owned(),
					format!("@{}", account.name),
				)
			})
			.collect();
		// Four rows fit without crowding the card; the rest scroll inside the list.
		let rows = saved.len().min(4) as f32;
		let height = rows * 54.0 + (rows - 1.0) * 6.0;
		let mut chosen = None;
		let mut forget = None;
		egui::ScrollArea::vertical()
			.id_salt("saved-accounts")
			.max_height(height)
			.min_scrolled_height(height)
			.show(ui, |ui| {
				ui.spacing_mut().item_spacing.y = 6.0;
				for (id, label, handle) in saved {
					let (row, remove) = ui
						.add_enabled_ui(enabled, |ui| {
							ui::design::account_row_with_remove(ui, &label, &handle, true)
						})
						.inner;
					if remove.is_some_and(|remove| remove.clicked()) {
						forget = Some(id);
					} else if row.clicked() {
						chosen = Some(id);
					}
				}
			});
		if let Some(id) = forget {
			self.confirming_forget = Some(id);
		} else if let Some(id) = chosen {
			self.messaging.switch_account_requested = Some(id);
		}
	}
	/// Forgetting is destructive (token, cached history, drafts), so it always asks first.
	fn confirm_forget_dialog(&mut self, ctx: &egui::Context) {
		let Some(account) = self.confirming_forget else {
			return;
		};
		let label = self
			.messaging
			.accounts
			.iter()
			.find(|saved| saved.id == account)
			.map(|saved| saved.label().to_owned())
			.unwrap_or_else(|| "this account".to_owned());
		let signed_in = self
			.state
			.user
			.as_ref()
			.is_some_and(|user| user.id == account);
		let confirm = ui::dialog::Confirm::new(
			"forget-account",
			format!("Forget {label}?"),
			if signed_in {
				"This is the account you are signed in with: you will be logged out, and its saved login, cached history and drafts on this device are removed."
			} else {
				"Its saved login, cached history and drafts on this device are removed. The Discord account itself is untouched; you can sign in again any time."
			},
		)
		.danger()
		.confirm_label("Forget account")
		.cancel_label("Keep");
		match confirm.show(ctx) {
			Some(ui::dialog::Choice::Confirmed) => {
				self.confirming_forget = None;
				self.forget_saved_account(ctx, account);
			}
			Some(ui::dialog::Choice::Cancelled) => self.confirming_forget = None,
			None => {}
		}
	}
	/// Owner authorization, with the token handling spelled out next to the checkbox.
	fn sign_in_consent(&mut self, ui: &mut egui::Ui) {
		let p = ui::design::palette(ui);
		egui::Frame::NONE
			.fill(p.base)
			.corner_radius(10)
			.inner_margin(egui::Margin::symmetric(12, 9))
			.show(ui, |ui| {
				ui.set_width(ui.available_width());
				ui.checkbox(
					&mut self.authorized,
					ui::design::medium(ui, "I own this account and authorize this session.", 13.0)
						.color(p.text_strong),
				);
				ui.add_space(4.0);
				ui.add(
					egui::Label::new(
						egui::RichText::new(
							"Passwords and 2FA stay on Discord's own login page; only the session token is kept, in your OS credential store.",
						)
						.size(12.0)
						.color(p.muted),
					)
					.wrap(),
				);
			});
	}
	/// One banner, only while something is happening or went wrong.
	fn sign_in_status(&mut self, ui: &mut egui::Ui) {
		let p = ui::design::palette(ui);
		let attention = matches!(
			self.state.auth,
			AuthState::Failed | AuthState::Expired | AuthState::Challenged
		);
		let busy = self.state.auth == AuthState::Authenticating
			|| self.forgetting
			|| self.cache_pending > 0
			|| self.cache_clears.pending();
		let show = attention
			|| busy || self.state.status != "Disconnected"
			|| (!self.fixture_only
				&& self.credential_status != "Checking saved login…"
				&& !self.credential_status.is_empty());
		if !show || (self.fixture_only && self.state.status == "Disconnected") {
			return;
		}
		ui.add_space(14.0);
		let (fill, stroke, color) = if attention {
			(
				p.warning.gamma_multiply(0.13),
				p.warning.gamma_multiply(0.45),
				p.warning,
			)
		} else {
			(p.raised, p.border, p.text)
		};
		egui::Frame::NONE
			.fill(fill)
			.stroke(egui::Stroke::new(1.0, stroke))
			.corner_radius(10)
			.inner_margin(egui::Margin::symmetric(12, 10))
			.show(ui, |ui| {
				ui.set_width(ui.available_width());
				ui.horizontal_top(|ui| {
					ui.spacing_mut().item_spacing.x = 9.0;
					let (icon, _) =
						ui.allocate_exact_size(egui::Vec2::splat(16.0), egui::Sense::hover());
					if attention {
						ui::icons::paint(
							ui.painter(),
							ui::icons::Icon::ShieldWarning,
							icon,
							p.warning,
						);
					} else if busy {
						ui.put(icon, egui::Spinner::new().size(14.0).color(p.muted));
					}
					ui.vertical(|ui| {
						ui.spacing_mut().item_spacing.y = 3.0;
						if self.state.status != "Disconnected" || attention {
							ui.add(
								egui::Label::new(
									egui::RichText::new(self.state.status)
										.size(13.0)
										.color(color),
								)
								.wrap(),
							);
						}
						if !self.fixture_only && !self.credential_status.is_empty() {
							ui.add(
								egui::Label::new(
									egui::RichText::new(self.credential_status)
										.size(12.0)
										.color(p.muted),
								)
								.wrap(),
							);
						}
						if self.cache_error || self.cache_pending > 0 || self.cache_clears.pending()
						{
							ui.add(
								egui::Label::new(
									egui::RichText::new(self.cache_status)
										.size(12.0)
										.color(p.muted),
								)
								.wrap(),
							);
						}
					});
				});
			});
	}
	/// Fixture-only entry into the offline preview, kept visually secondary to signing in.
	#[cfg(feature = "demo")]
	fn sign_in_preview(&mut self, ui: &mut egui::Ui) {
		let p = ui::design::palette(ui);
		ui.add_space(18.0);
		ui.horizontal(|ui| {
			let y = ui.cursor().top() + 8.0;
			let left = ui.cursor().left();
			let width = ui.available_width();
			let galley =
				ui.painter()
					.layout_no_wrap("or".into(), egui::FontId::proportional(12.0), p.muted);
			let text_w = galley.size().x + 20.0;
			ui.painter().hline(
				left..=left + (width - text_w) * 0.5,
				y,
				egui::Stroke::new(1.0, p.border),
			);
			ui.painter().galley(
				egui::pos2(
					left + (width - galley.size().x) * 0.5,
					y - galley.size().y * 0.5,
				),
				galley,
				p.muted,
			);
			ui.painter().hline(
				left + (width + text_w) * 0.5..=left + width,
				y,
				egui::Stroke::new(1.0, p.border),
			);
			ui.allocate_space(egui::vec2(width, 16.0));
		});
		ui.add_space(14.0);
		if ui::design::secondary_button(ui, "Explore the offline preview").clicked() {
			if let Some(store) = &mut self.store {
				store.cancel_load();
			}
			self.connection = None;
			self.pending_save = None;
			self.pending_account_save = None;
			let generation = self.state.generation + 1;
			self.state = test_support::demo_state();
			test_support::seed_demo_folder_mosaic(&mut self.state);
			test_support::seed_access_marks(&mut self.state);
			self.state.generation = generation;
			self.messaging.clear();
			self.messaging.show_hidden_channels = true;
		}
		ui.add_space(8.0);
		ui.vertical_centered(|ui| {
			ui.label(
				egui::RichText::new("Sample conversations. No Discord connection.")
					.size(12.0)
					.color(p.muted),
			);
		});
	}
	/// Secondary panels: what this client is, and the owner's own session token.
	fn sign_in_disclosures(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
		let p = ui::design::palette(ui);
		if ui::design::disclosure(ui, "About Serein", self.about_open).clicked() {
			self.about_open = !self.about_open;
		}
		if self.about_open {
			ui.add_space(2.0);
			egui::Frame::NONE
				.inner_margin(egui::Margin {
					left: 30,
					right: 4,
					top: 2,
					bottom: 8,
				})
				.show(ui, |ui| {
					ui.spacing_mut().item_spacing.y = 6.0;
					for line in [
						"Messaging, reactions, search and read markers have offline tests. Real Discord interoperability is still unverified; attachment uploads and advanced search remain incomplete.",
						"Messages and drafts are cached locally. Login tokens use the operating system credential store.",
						"Unofficial clients may put your Discord account at risk.",
					] {
						ui.add(
							egui::Label::new(
								egui::RichText::new(line).size(12.0).color(p.muted),
							)
							.wrap(),
						);
					}
					if !self.fixture_only {
						ui.add_space(2.0);
						if ui::design::button(ui, "Forget saved login", ui::design::ButtonKind::Outline)
							.clicked()
						{
							self.logout(ctx);
						}
					}
				});
		}
		if ui::design::disclosure(ui, "Sign in with a session token", self.token_open).clicked() {
			self.token_open = !self.token_open;
		}
		if self.token_open {
			ui.add_space(2.0);
			egui::Frame::NONE
				.inner_margin(egui::Margin {
					left: 30,
					right: 4,
					top: 2,
					bottom: 6,
				})
				.show(ui, |ui| {
					ui.add(
						egui::Label::new(
							egui::RichText::new(
								"For owners who already hold a valid Discord session token, for example from another signed-in Serein install. Passwords and 2FA are never used here; this bypasses Discord's hosted login page entirely.",
							)
							.size(12.0)
							.color(p.muted),
						)
						.wrap(),
					);
					ui.add_space(8.0);
					ui::design::input(
						ui,
						egui::TextEdit::singleline(&mut *self.token_input)
							.password(true)
							.char_limit(2048)
							.hint_text("Session token"),
					);
					ui.add_space(8.0);
					let connect = ui
						.add_enabled_ui(self.authorized && !self.token_input.is_empty(), |ui| {
							ui::design::button(
								ui,
								"Connect with this token",
								ui::design::ButtonKind::Primary,
							)
						})
						.inner;
					if connect.clicked() && !self.fixture_only {
						let input = std::mem::take(&mut *self.token_input);
						match SessionSecret::from_owner_input(input) {
							Ok(secret) => self.connect(secret, true, ctx),
							Err(failure) => self.state.status = failure.label(),
						}
					}
				});
		}
	}
	fn clear_avatars(&mut self, ctx: &egui::Context) {
		self.avatar_start_failed = false;
		if self.avatar_cleanup.is_some() && !self.fixture_only && !self.state.demo {
			self.avatar_clear_account = self.state.user.as_ref().map(|user| user.id);
		}
		// Logout may follow expiry, when the downloading worker was already stopped.
		if self.avatars.is_none()
			&& self.avatar_cleanup.is_none()
			&& !self.fixture_only
			&& !self.state.demo
			&& let Some(user) = &self.state.user
		{
			match avatars::AvatarWorker::start(&self.runtime, user.id, ctx.clone()) {
				Ok(worker) => self.avatars = Some(worker),
				Err(error) => {
					self.cache_error = true;
					self.cache_status = error;
				}
			}
		}
		if let Some(worker) = self.avatars.take() {
			self.avatar_cleanup = Some(worker.shutdown_and_clear());
		}
		self.messaging.clear_avatars();
	}
	fn poll_avatars(&mut self, ctx: &egui::Context) {
		if let Some(cleanup) = &self.avatar_cleanup {
			match cleanup.try_recv() {
				Ok(result) => {
					self.avatar_cleanup = None;
					if let Err(error) = result {
						self.cache_error = true;
						self.cache_status = error;
					}
				}
				Err(std::sync::mpsc::TryRecvError::Disconnected) => {
					self.avatar_cleanup = None;
					self.cache_error = true;
					self.cache_status =
						"Avatar cache cleanup failed; cached pictures may remain on disk";
				}
				Err(std::sync::mpsc::TryRecvError::Empty) => {}
			}
		}
		if self.avatar_cleanup.is_none()
			&& let Some(account) = self.avatar_clear_account.take()
		{
			match avatars::AvatarWorker::start(&self.runtime, account, ctx.clone()) {
				Ok(worker) => self.avatar_cleanup = Some(worker.shutdown_and_clear()),
				Err(error) => {
					self.cache_error = true;
					self.cache_status = error;
				}
			}
		}
		if self.fixture_only || self.state.demo || self.state.auth != AuthState::Authenticated {
			if let Some(worker) = self.avatars.take() {
				self.avatar_cleanup = Some(worker.shutdown());
			}
			return;
		}
		if !self.avatar_start_failed
			&& self.avatars.is_none()
			&& self.avatar_cleanup.is_none()
			&& let Some(user) = &self.state.user
		{
			match avatars::AvatarWorker::start(&self.runtime, user.id, ctx.clone()) {
				Ok(worker) => {
					self.messaging.clear_avatars();
					self.avatars = Some(worker);
				}
				Err(error) => {
					self.avatar_start_failed = true;
					self.cache_error = true;
					self.cache_status = error;
				}
			}
		}
		if let Some(worker) = &mut self.avatars {
			for _ in 0..32 {
				let Some(result) = worker.poll() else {
					break;
				};
				if let Some(error) = result.error {
					self.cache_error = true;
					self.cache_status = error;
				}
				let stage = result.stage;
				self.messaging
					.accept_avatar(ctx, result.key.clone(), result.image);
				if stage == avatars::DecodeStage::Settled {
					self.messaging
						.accept_gif_animation(result.key, result.frames);
				}
			}
		}
	}
	fn poll_voice(&mut self, ctx: &egui::Context) {
		if let Some(command) =
			self.voice
				.poll(&self.runtime, &mut self.state, &mut self.messaging, ctx)
		{
			self.command(command);
		}
	}
	fn poll(&mut self, ctx: &egui::Context) {
		let mut cached = Vec::new();
		let mut presence_cache_stopped = false;
		if let Some(cache) = &self.cache {
			for _ in 0..16 {
				match cache.receive.try_recv() {
					Ok(value) => cached.push(value),
					Err(std::sync::mpsc::TryRecvError::Disconnected) => {
						if self.messaging.custom_font.busy && self.font_picker.is_none() {
							self.messaging.custom_font.busy = false;
							self.messaging.custom_font.status =
								"Local storage worker stopped. Restart Serein to save fonts.";
						}
						if self.messaging.channel_preferences_reload
							|| self.messaging.channel_preferences_load_pending
						{
							self.messaging.channel_preferences_reload = false;
							self.messaging.channel_preferences_load_pending = false;
							self.messaging.channel_preferences_status = "Local storage worker stopped; restart Serein to restore channel preferences.";
						}
						presence_cache_stopped = self.presence_load_pending;
						break;
					}
					Err(std::sync::mpsc::TryRecvError::Empty) => break,
				}
			}
		}
		if presence_cache_stopped {
			self.presence_load_pending = false;
			self.flush_deferred_connect(ctx);
		}
		for (generation, outcome, _reservation) in cached {
			self.cache_pending = self.cache_pending.saturating_sub(1);
			// Settings are global; account removal/write failures still matter after logout.
			match &outcome {
				cache::Outcome::CustomFont(result) => {
					self.accept_font(ctx, result);
					continue;
				}
				cache::Outcome::AppPreferences(result) => {
					self.app_settings.loaded = result.is_ok();
					if !self.app_settings.state.touched {
						match result {
							Ok(value) => self.app_settings.current = value.as_ref().clone(),
							Err(_) => self.app_settings.state.failed = true,
						}
						if !self.state.demo && !self.fixture_only {
							self.app_settings.apply(&mut self.messaging);
						}
					}
					continue;
				}
				cache::Outcome::AppPreferencesSaved(result) => {
					self.app_settings.state.saving = false;
					self.app_settings.state.failed = result.is_err();
					continue;
				}
				cache::Outcome::MinimizeToTray(result) => {
					self.tray_setting.restore(*result);
					if !self.fixture_only {
						self.messaging.minimize_to_tray = self.tray_setting.enabled;
					}
					continue;
				}
				cache::Outcome::MinimizeToTraySaved(result) => {
					self.tray_setting.saving = false;
					self.tray_setting.failed = result.is_err();
					continue;
				}
				cache::Outcome::GameActivity(result) => {
					self.game_activity.restore(*result);
					if !self.state.demo && !self.fixture_only {
						self.messaging.share_game_activity = self.game_activity.enabled;
					}
					continue;
				}
				cache::Outcome::GameActivitySaved(result) => {
					self.game_activity.saving = false;
					self.game_activity.failed = result.is_err();
					continue;
				}
				cache::Outcome::ReadingPreferences(result) => {
					if let Some(value) = self.reading.restore(*result)
						&& !self.state.demo
						&& !self.fixture_only
					{
						self.messaging.apply_reading_preferences(ctx, value);
					}
					continue;
				}
				cache::Outcome::ReadingPreferencesSaved(result) => {
					self.reading.saved(*result);
					continue;
				}
				cache::Outcome::Appearance(appearance, variant) => {
					if !self.state.demo && !self.appearance_changed {
						self.appearance = match appearance {
							local_store::Appearance::System => egui::ThemePreference::System,
							local_store::Appearance::Light => egui::ThemePreference::Light,
							local_store::Appearance::Dark => egui::ThemePreference::Dark,
						};
						ctx.set_theme(self.appearance);
					}
					if !self.state.demo && !self.variant_changed {
						// Unknown keys from a newer build fall back to the default preset.
						let variant = variant
							.as_deref()
							.and_then(ui::design::Variant::from_key)
							.unwrap_or_default();
						ui::design::set_variant(variant);
						ui::design::apply(ctx);
					}
					continue;
				}
				cache::Outcome::Failed {
					error,
					message,
					draft_restore,
					history_cleanup,
				} => {
					if *history_cleanup && let Some(cache) = &self.cache {
						self.cache_clears.acknowledge(&cache.history);
					}
					let _ = error;
					self.cache_error = true;
					self.cache_status = message;
					if *draft_restore && generation == self.state.generation {
						self.messaging.draft_restore_pending = false;
						self.state.drafts.retain(|_, content| !content.is_empty());
					}
					continue;
				}
				cache::Outcome::Accounts { roster, pruned } => {
					self.roster_pending = false;
					// Pruning is already committed, so clean up regardless of the re-read.
					for account in pruned {
						if let Some(store) = &self.store {
							let _ = store.send.try_send((
								self.state.generation,
								credentials::Request::NONE,
								credentials::Operation::ForgetAccount(*account),
							));
						}
						self.queue_cache_for(*account, cache::Operation::Forget);
					}
					match roster {
						Ok(accounts) => self.messaging.accounts = accounts.clone(),
						Err(_) => {
							self.roster_failed = true;
							self.cache_error = true;
							self.cache_status = "Could not read or update the saved account list";
						}
					}
					continue;
				}
				cache::Outcome::AccountPresences(result) => {
					self.presence_load_pending = false;
					if let Ok(rows) = result {
						self.account_presences.clone_from(rows);
					}
					self.flush_deferred_connect(ctx);
					continue;
				}
				cache::Outcome::AccountPresenceSaved(result) => {
					if result.is_err() {
						self.presence_saved = None;
					}
					continue;
				}
				cache::Outcome::HistoryCleared => {
					if let Some(cache) = &self.cache
						&& self.cache_clears.acknowledge(&cache.history)
						&& !self.cache_error
					{
						if cache.history.allows(cache.history.epoch()) {
							self.cache_status = "Cached history cleared; saved drafts preserved";
						} else {
							self.cache_status = "Requested history cleanup completed; history cache remains disabled until restart after a storage failure";
						}
					}
					continue;
				}
				_ => {}
			}
			if generation != self.state.generation {
				continue;
			}
			match outcome {
				cache::Outcome::ChannelPreferences(result) => {
					self.messaging.channel_preferences_load_pending = false;
					self.messaging.channel_preferences_reload = false;
					match result {
						Ok(preferences) => self.messaging.restore_channel_preferences(preferences),
						Err(error) => {
							self.messaging.channel_preferences_status = match error {
								local_store::StoreError::Incompatible => {
									"Saved channel preferences are damaged or incompatible with this build."
								}
								_ => "Could not read channel preferences from local storage.",
							};
						}
					}
				}
				cache::Outcome::ChannelPreferencesSaved(result) => {
					self.messaging.channel_preferences_save_pending = false;
					self.messaging.channel_preferences_status = if result.is_ok() {
						""
					} else {
						"Could not save channel preferences."
					};
				}
				cache::Outcome::GifFavorites(favorites) => {
					self.state.restore_gif_favorites(favorites);
				}
				cache::Outcome::Drafts(drafts) => {
					for (channel, content) in drafts {
						if !self
							.state
							.pending
							.iter()
							.any(|pending| pending.channel == channel)
						{
							self.state.drafts.entry(channel).or_insert(content);
						}
					}
					self.messaging.draft_restore_pending = false;
					self.state.drafts.retain(|_, content| !content.is_empty());
					if !self.cache_error {
						self.cache_status = "Saved drafts restored; check the conversation before resending recovered text";
					}
				}
				outcome @ cache::Outcome::Channel { .. } => {
					if let Some(cache) = &self.cache {
						hydrate_cache_result(&mut self.state, &cache.history, outcome);
					}
				}
				cache::Outcome::Saved => {
					if !self.cache_error {
						self.cache_status = "Local changes saved";
					}
				}
				cache::Outcome::Appearance(..)
				| cache::Outcome::CustomFont(_)
				| cache::Outcome::AppPreferences(_)
				| cache::Outcome::AppPreferencesSaved(_)
				| cache::Outcome::MinimizeToTray(_)
				| cache::Outcome::MinimizeToTraySaved(_)
				| cache::Outcome::GameActivity(_)
				| cache::Outcome::GameActivitySaved(_)
				| cache::Outcome::ReadingPreferences(_)
				| cache::Outcome::ReadingPreferencesSaved(_)
				| cache::Outcome::Accounts { .. }
				| cache::Outcome::HistoryCleared
				| cache::Outcome::Failed { .. }
				| cache::Outcome::AccountPresences(_)
				| cache::Outcome::AccountPresenceSaved(_) => unreachable!(),
			}
		}
		self.retry_history_clears();
		let mut results = Vec::new();
		if let Some(store) = &mut self.store {
			for _ in 0..4 {
				match store.poll(std::time::Instant::now()) {
					Some(result) => results.push(result),
					None => break,
				}
			}
			if let Some(remaining) = store.remaining(std::time::Instant::now()) {
				ctx.request_repaint_after(remaining);
			}
		}
		for (generation, outcome) in results {
			if generation != self.state.generation {
				continue;
			}
			match outcome {
				credentials::Outcome::Loaded(result) => {
					let switching = self.switching.take();
					if let Ok(Some(secret)) = result {
						// `connect()` already sets a "Connecting to Discord…" status; avoid
						// stacking a second, near-duplicate line under the restore screen.
						self.credential_status = "";
						self.connect(secret, switching.is_some(), ctx);
					} else {
						self.credential_status = credentials::loaded_status(&result);
						if let Some(account) = switching {
							// The entry stays: the owner decides whether to forget it. Signing
							// in again with "Use another account" refreshes its token, which the
							// roster must expect again.
							self.queue_cache_for(
								model::Id(0),
								cache::Operation::SetAccountToken {
									account,
									has_token: false,
								},
							);
							self.credential_status = "No saved login for that account on this device. Use another account to sign in again, or forget it with ×.";
							self.messaging.toasts.push(
								ui::design::Level::Warning,
								"That account's saved login is missing; sign in again to refresh it",
							);
						}
					}
				}
				credentials::Outcome::AccountSaved(account, result) => {
					if result.is_ok() {
						self.queue_cache_for(
							model::Id(0),
							cache::Operation::SetAccountToken {
								account,
								has_token: true,
							},
						);
					} else if result != Err(platform::CredentialError::NoStore) {
						self.credential_status =
							"Could not save this account for the switcher; sign in again to retry";
					}
				}
				credentials::Outcome::Saved(Ok(())) => {
					self.credential_status = "Login saved in the OS credential store"
				}
				credentials::Outcome::Saved(Err(platform::CredentialError::NoStore)) => {
					self.messaging.toasts.push(
						ui::design::Level::Warning,
						"No OS keyring found, so you will need to sign in again next launch",
					);
				}
				credentials::Outcome::Saved(Err(_)) => {
					self.credential_status =
						"Could not save login; this session will not restore automatically"
				}
				credentials::Outcome::AccountForgotten(result) => {
					if result.is_err_and(|error| error != platform::CredentialError::NoStore) {
						self.messaging.toasts.push(
							ui::design::Level::Error,
							"Could not remove that account's saved login from the OS credential store",
						);
					}
				}
				credentials::Outcome::Forgotten(result) => {
					self.forgetting = false;
					self.credential_status = match result {
						Ok(()) => "Saved login removed",
						// Nothing could have been saved without a credential store.
						Err(platform::CredentialError::NoStore) => "",
						Err(_) => {
							"Could not remove saved login; remove cz.viceverse.serein / discord-session in your OS credential manager"
						}
					};
				}
			}
		}
		let mut events = Vec::new();
		let mut terminal = None;
		if let Some(connection) = &mut self.connection {
			for _ in 0..client_core::EVENT_SLOTS {
				match connection.events.try_recv() {
					Ok(event) => events.push(event),
					Err(_) => break,
				}
			}
			// Collect reliable events first: their preceding typing signals are now queued.
			// Apply typing first so messages/access changes retire those older signals.
			let reliable_count = events.len();
			for _ in 0..8 {
				match connection.typing.try_recv() {
					Ok(event) => events.push(event),
					Err(_) => break,
				}
			}
			let typing_count = events.len() - reliable_count;
			events.rotate_right(typing_count);
			terminal = *connection.terminal.borrow();
		}
		let mut persist_timeline = false;
		let mut full_window = false;
		let mut changed_messages = std::collections::BTreeSet::new();
		for mut event in events {
			if event.generation != self.state.generation {
				continue;
			}
			let user_action_notice = user_action_notice(&event.event);
			let user_action_was_pending =
				user_action_notice.is_some() && self.state.user_action_pending();
			self.delete_cached_messages(&event.event);
			match &event.event {
				Event::Delete { channel, id } => {
					self.messaging
						.messages_deleted(ctx, *channel, std::slice::from_ref(id));
				}
				Event::DeleteBulk { channel, ids } if ids.len() <= 100 => {
					self.messaging.messages_deleted(ctx, *channel, ids);
				}
				_ => {}
			}
			let voice_failure = self.voice.observe(&self.state, &mut event.event);
			let ready = event.event.ready_navigation().is_some();
			let resumed = matches!(event.event, Event::Resumed);
			let confirmed_channel = confirmed_recovery_channel(&self.state, &event.event);
			let deleted_shortcut = match &event.event {
				Event::Unavailable(channel)
				| Event::ThreadRemoved { id: channel, .. }
				| Event::ChannelAction(client_core::channel_actions::Event::Finished {
					channel,
					result: Ok(client_core::channel_actions::Outcome::Deleted),
					..
				}) => Some(*channel),
				_ => None,
			};
			let mut removed_channels = access_candidates(&self.state, &event.event);
			removed_channels.retain(|id| self.state.can_read_history(*id));
			let invalidate = matches!(
				event.event,
				Event::Resync | Event::PermissionsChanged | Event::Unavailable(_)
			) || matches!(&event.event, Event::RecipientRemoved { user, .. } if self.state.user.as_ref().is_some_and(|owner| owner.id == *user))
				|| matches!(&event.event, Event::Reactions(client_core::reactions::Event::Read {channel,result:Err(Failure::Forbidden),..}) if self.state.selected==Some(*channel))
				|| matches!(&event.event, Event::HistoryFailed { channel, request, failure: Failure::Forbidden }
                if self.state.selected == Some(*channel) && self.state.request == *request && self.state.history_pending);
			let history_changed = changes_active_history(&self.state, &event.event);
			if history_changed {
				match &event.event {
					Event::Message(message)
					| Event::SendResult {
						result: Ok(message),
						..
					} => {
						changed_messages.insert(message.id);
					}
					Event::ServerAction(client_core::server_actions::Event::InviteSent {
						result: Ok(sent),
						..
					}) => {
						changed_messages.insert(sent.1.id);
					}
					Event::Edited { message, .. } => {
						changed_messages.insert(*message);
					}
					Event::Patch(patch) => {
						changed_messages.insert(patch.id);
					}
					_ => full_window = true,
				}
			}
			let guild_ack = matches!(
				&event.event,
				Event::ReadState(client_core::read_state::Event::GuildAck {
					guild,
					request,
					result: Ok(()),
				}) if self.state.pending_guild_ack(*guild, *request)
			);
			if event.generation == self.state.generation
				&& (invalidate
					|| event.event.changes_access()
					|| guild_ack || matches!(
					&event.event,
					Event::NotificationPreferences(_)
						| Event::ChannelAction(_)
						| Event::UserAction(_)
						| Event::Disconnected
						| Event::ReadState(client_core::read_state::Event::Ack { .. })
						| Event::ReadState(client_core::read_state::Event::Result {
							result: Ok(()),
							..
						})
				)) {
				self.notifications.dismiss();
			}
			let data_changes = extension_data_events::Changes::capture(&self.state, &event);
			let extension_events = if self.extensions.has_message_events(&self.state) {
				extension_events::capture(&self.state, &event.event)
			} else {
				Vec::new()
			};
			if event.generation == self.state.generation
				&& (event.event.changes_access()
					|| matches!(event.event, Event::Disconnected)
					|| matches!(&event.event, Event::HistoryFailed { channel, request, failure: Failure::Forbidden }
						if Some(*channel) == self.state.selected && *request == self.state.request && self.state.history_pending))
			{
				self.extensions.access_changed(&mut self.messaging);
			}
			self.state.apply(event);
			self.extensions.data_changed(data_changes);
			self.extensions.cancel_stale_message_events(&self.state);
			for candidate in extension_events {
				if let Some(event) = candidate.admit(&self.state) {
					self.extensions.message_event(&self.state, event);
				}
			}
			if user_action_was_pending
				&& !self.state.user_action_pending()
				&& let Some((level, text)) = user_action_notice
			{
				self.messaging.toasts.push(level, text);
			}
			// Only admitted service messages can establish a deleted reply target.
			// Fence pending disk writes before the post-drain timeline snapshot is saved.
			let deleted_replies = self.state.take_reply_deletions();
			if let Some(&(channel, _)) = deleted_replies.first() {
				full_window = true;
				let ids: Vec<_> = deleted_replies.into_iter().map(|(_, id)| id).collect();
				self.messaging.messages_deleted(ctx, channel, &ids);
				self.delete_cached_ids(channel, ids);
			}
			removed_channels.retain(|id| !self.state.can_read_history(*id));
			for channel in removed_channels.iter().copied().chain(deleted_shortcut) {
				if self.state.channel(channel).is_none() {
					self.messaging.channel_preferences_changed |=
						self.messaging.channel_preferences.forget(channel);
				}
			}
			if let Some(error) = voice_failure
				&& let Some(command) = self.voice.fail(&mut self.state, error)
			{
				self.command(command);
			}
			// ponytail: accepted navigation removals clear account-wide history;
			// add scoped disk deletion if channel churn makes refetch cost significant.
			if invalidate || !removed_channels.is_empty() {
				self.queue_cache(cache::Operation::ClearHistory);
			}
			persist_timeline |= history_changed;
			if let Some(channel) = confirmed_channel {
				let content = recovery_draft(&self.state, channel);
				self.queue_cache(cache::Operation::SaveDraft { channel, content });
			}
			if ready && self.state.auth == AuthState::Authenticated {
				// The worker survives logout; each accepted account READY restores its own drafts.
				if !self.messaging.draft_restore_pending {
					self.messaging.draft_restore_pending =
						self.queue_cache(cache::Operation::LoadDrafts);
					self.queue_cache(cache::Operation::LoadGifFavorites);
				}
				if !self.messaging.channel_preferences_loaded
					&& !self.messaging.channel_preferences_load_pending
				{
					self.messaging.channel_preferences_reload = true;
				}
				if let Some(store) = &self.store {
					// The active entry restores on launch; the per-account entry backs the switcher.
					let mut queued = true;
					// A token the owner just supplied replaces whatever was stored before.
					let fresh_token = self.pending_save.is_some();
					if let Some(secret) = self.pending_save.take() {
						queued &= store
							.send
							.try_send((
								self.state.generation,
								credentials::Request::NONE,
								credentials::Operation::Save(secret),
							))
							.is_ok();
					}
					// Writing an existing entry is an access-controlled keychain operation on
					// macOS, so only write when the roster says none exists or the token is new.
					if let Some(secret) = self.pending_account_save.take()
						&& let Some(account) = self.state.user.as_ref().map(|user| user.id)
						&& (fresh_token
							|| !self
								.messaging
								.accounts
								.iter()
								.any(|saved| saved.id == account && saved.has_token))
					{
						queued &= store
							.send
							.try_send((
								self.state.generation,
								credentials::Request::NONE,
								credentials::Operation::SaveAccount(account, secret),
							))
							.is_ok();
					}
					if !queued {
						self.credential_status = "Could not queue saved login; session only";
					}
				}
			}
			if (ready || resumed)
				&& self.state.auth == AuthState::Authenticated
				&& self.state.selected.is_some_and(|selected| {
					self.state
						.channels
						.iter()
						.any(|channel| channel.id == selected && channel.supports_text())
				}) {
				let command = self.state.history(None);
				self.command(command);
			}
		}
		if persist_timeline
			&& self.state.freshness == model::Freshness::Fresh
			&& let Some(channel) = self.state.selected
			&& self.state.can_read_history(channel)
		{
			let messages = self
				.state
				.timeline
				.iter()
				.filter(|m| full_window || changed_messages.contains(&m.id))
				.cloned()
				.collect();
			let operation = if full_window {
				cache::Operation::SaveChannel { channel, messages }
			} else {
				cache::Operation::SaveChanges {
					channel,
					messages,
					retained: self.state.timeline.iter().map(|m| m.id).collect(),
				}
			};
			self.queue_cache(operation);
		}
		if let Some(failure) = terminal {
			for (setting, scope) in [
				("SEREIN_MEMBER_DIAGNOSTICS", "members"),
				("SEREIN_GATEWAY_DIAGNOSTICS", "gateway"),
			] {
				if std::env::var_os(setting).as_deref() == Some(std::ffi::OsStr::new("1")) {
					use std::io::Write;
					// One extra fixed-label terminal line per enabled scope; closed stderr is OK.
					let _ = writeln!(
						std::io::stderr(),
						"[Serein {scope}] Session stopped: {}",
						failure.label()
					);
				}
			}
			self.connection = None;
			self.pending_save = None;
			self.pending_account_save = None;
			self.state.apply(Envelope {
				generation: self.state.generation,
				event: Event::Failure(failure),
			});
			if !matches!(self.state.auth, AuthState::Expired | AuthState::Challenged) {
				self.state.auth = AuthState::Failed;
			}
			for pending in &mut self.state.pending {
				if pending.delivery == Delivery::Sending {
					pending.delivery = Delivery::Ambiguous;
				}
			}
			if failure == Failure::Expired
				&& let Some(store) = &self.store
			{
				let _ = store.send.try_send((
					self.state.generation,
					credentials::Request::NONE,
					credentials::Operation::Forget,
				));
			}
		}
		if let Some(login) = &self.login {
			login.pump();
			if let Some(secret) = login.token() {
				self.connect(secret, true, ctx);
			} else if login.expired() {
				let crashed = login.crashed();
				self.login = None;
				self.state.auth = AuthState::Challenged;
				self.state.status = if crashed {
					"Login window stopped unexpectedly (web process ended); no session accepted"
				} else {
					"Login timed out or token handoff unavailable; no session accepted"
				};
			}
		}
		// Network and store workers request repaint only when their outcomes change.
		if let Some(connection) = &self.connection {
			connection.set_typing_channel(self.state.typing_scope());
		}
		if !self.state.demo
			&& let Some(command) = self.state.next_reaction_read()
		{
			self.command(command);
		}

		#[cfg(target_os = "linux")]
		if self.login.is_some() {
			ctx.request_repaint_after(self.frame_period().unwrap_or(Duration::from_millis(16)));
		}
		if self.login.is_some() {
			ctx.request_repaint_after(Duration::from_secs(1));
		}
	}
}
impl Desktop {
	fn sync_customization(&mut self, ctx: &egui::Context) {
		if self.messaging.primary_color != ui::design::primary_color() {
			ui::design::set_primary_color(self.messaging.primary_color);
			ui::design::apply(ctx);
			ctx.request_repaint();
		}
		let effects = (
			self.transparency_available,
			self.messaging.transparency,
			self.messaging.blur,
			self.messaging.transparent_all,
		);
		if effects != ui::design::default_window_effects() {
			ui::design::set_window_effects(effects.0, effects.1, effects.2, effects.3);
			ui::design::apply(ctx);
			ctx.request_repaint();
		}
	}
	fn sync_window_effects(&mut self) {
		if !self.transparency_available {
			return;
		}
		let effects = ui::design::window_effects();
		let transparent = effects.0 && effects.1 > 0;
		if transparent != self.window_transparent {
			self.window.set_transparent(transparent);
			self.window_transparent = transparent;
		}
		let blur = transparent && effects.2 > 0;
		if let Some(window_blur) = &mut self.window_blur {
			window_blur.set_enabled(blur);
		}
	}
	/// Frame period of the display the window is on; egui otherwise assumes 60 Hz.
	fn frame_period(&self) -> Option<Duration> {
		self.monitor_period
	}
	fn refresh_frame_period(&mut self) -> Option<Duration> {
		let millihertz = self.window.current_monitor()?.refresh_rate_millihertz()?;
		(1_000..=1_000_000)
			.contains(&millihertz)
			.then(|| Duration::from_secs_f64(1000.0 / f64::from(millihertz)))
	}
}
impl eframe::App for Desktop {
	fn clear_color(&self, visuals: &egui::Visuals) -> [f32; 4] {
		if self.window_transparent {
			egui::Color32::TRANSPARENT.to_normalized_gamma_f32()
		} else {
			// Opaque windows need opaque pixels too, including uncovered panel corners.
			visuals.panel_fill.to_opaque().to_normalized_gamma_f32()
		}
	}
	fn persist_egui_memory(&self) -> bool {
		false
	}
	fn raw_input_hook(&mut self, ctx: &egui::Context, raw_input: &mut egui::RawInput) {
		// Viewport position/scale comes from native events; avoid an OS monitor query on paints.
		if let Some(viewport) = raw_input.viewports.get(&raw_input.viewport_id) {
			let geometry = (viewport.outer_rect, viewport.native_pixels_per_point);
			if self.monitor_geometry != Some(geometry) {
				self.monitor_geometry = Some(geometry);
				self.monitor_period = self.refresh_frame_period();
			}
		}
		if let Some(period) = self.frame_period() {
			raw_input.predicted_dt = period.as_secs_f32();
		}
		let track = self.messaging.tracking_pointer();
		let intercepted =
			self.pointer
				.intercept(raw_input, &self.window, ctx.pixels_per_point(), track);
		self.messaging.middle_button(intercepted.middle);
		self.messaging.side_buttons(intercepted.side);
		if track
			&& let Some(egui::Event::PointerMoved(pos)) = raw_input.events.last()
			&& !ctx.content_rect().contains(*pos)
		{
			ctx.request_repaint();
		}
	}
	fn on_exit(&mut self) {
		if self.updater.finish_restart().is_err() {
			eprintln!(
				"Could not hand off the prepared update. The installed app was not replaced."
			);
		}
	}
	/// One UI frame: pumps workers, expires challenges, renders and drains commands.
	fn logic(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
		let search_focused = {
			#[cfg(feature = "demo")]
			{
				self.frame_metrics.sample.is_some() && self.messaging.friends_sample_focused(ctx)
			}
			#[cfg(not(feature = "demo"))]
			{
				false
			}
		};
		self.frame_metrics.begin(ctx, search_focused);
		self.startup.sync(
			ctx,
			&self.runtime,
			&mut self.messaging,
			self.fixture_only || self.state.demo,
		);
		self.messaging.sync_reading_zoom(ctx);
		self.poll(ctx);
		self.sync_fonts(ctx);
		self.hotkeys.sync(&self.messaging.keybinds, &self.runtime);
		self.messaging.global_keybind_status = self.hotkeys.status();
		self.hotkeys.poll();
		let voice_toggles = self.hotkeys.take_toggle_pending()
			| self
				.messaging
				.voice_toggle_pressed(ctx, self.hotkeys.global_toggle_mask());
		if voice_toggles != 0 && !self.fixture_only {
			let mut muted = self.messaging.voice_muted;
			let mut deafened = self.messaging.voice_deafened;
			let mut mic_toggled = false;
			let mut deaf_toggled = false;
			if voice_toggles & 1 != 0 {
				muted = !muted;
				mic_toggled = true;
			}
			if voice_toggles & 2 != 0 {
				deafened = !deafened;
				deaf_toggled = true;
			}
			self.messaging.voice_muted = muted;
			self.messaging.voice_deafened = deafened;
			if deaf_toggled {
				let cue = if deafened {
					model::notification_preferences::Sound::Deafen
				} else {
					model::notification_preferences::Sound::Undeafen
				};
				if self.messaging.notification_options.allows(cue) {
					self.messaging.notification_preview = Some(cue);
				}
			} else if mic_toggled {
				let cue = if muted {
					model::notification_preferences::Sound::Mute
				} else {
					model::notification_preferences::Sound::Unmute
				};
				if self.messaging.notification_options.allows(cue) {
					self.messaging.notification_preview = Some(cue);
				}
			}
			if self.state.auth == AuthState::Authenticated
				&& let Some(command) = self.state.set_call_mute(muted, deafened)
				&& !self.state.demo
			{
				self.command(command);
			}
		}
		if self.updater.sync(
			ctx,
			&self.runtime,
			&mut self.messaging.updates,
			!cfg!(debug_assertions)
				&& !self.fixture_only
				&& !self.state.demo
				&& (self.app_settings.loaded || self.app_settings.state.touched),
		) {
			// Installing needs a real exit, so this close must not stop at the tray.
			self.tray_window.quit(ctx);
		}
		self.state.expire_interaction(std::time::Instant::now());
		if self.state.interactions.busy() {
			ctx.request_repaint_after(Duration::from_millis(250));
		}
		self.poll_interaction_files(ctx);
		self.state.expire_verification();
		if self.state.verification().is_some() {
			ctx.request_repaint_after(Duration::from_secs(1));
		}
		if self.login.is_some()
			|| self.state.auth != AuthState::Authenticated
			|| ctx.input(|input| input.viewport().close_requested())
		{
			self.captcha.close();
			self.messaging.verification.active = false;
		}
		self.extensions.tick(
			&mut self.state,
			&mut self.messaging,
			ctx,
			&self.runtime,
			&self.window,
			self.fixture_only,
		);
		self.sync_customization(ctx);
		#[cfg(feature = "demo")]
		if self.demo_typing
			&& let Some(channel) = self.state.selected
		{
			let wall = std::time::SystemTime::now();
			let timestamp = wall
				.duration_since(std::time::UNIX_EPOCH)
				.map(|age| age.as_secs())
				.unwrap_or_default();
			for user in [2, 3] {
				self.state.observe_typing_at(
					client_core::typing::Signal {
						channel,
						user: model::Id(user),
						timestamp,
					},
					wall,
					std::time::Instant::now(),
				);
			}
		}
		let mut tray_events = Vec::with_capacity(4);
		if let Some(tray) = &mut self.tray {
			// The Linux worker can refill event bits while this frame consumes them.
			for _ in 0..4 {
				let Some(event) = tray.take_event() else {
					break;
				};
				tray_events.push(event);
			}
		}
		#[cfg(target_os = "linux")]
		let quitting = tray_events.contains(&platform::tray::Event::Quit);
		for event in tray_events {
			match event {
				platform::tray::Event::Quit => {
					self.tray_window.quit(ctx);
				}
				platform::tray::Event::Show => self.tray_window.show(ctx),
				#[cfg(target_os = "linux")]
				platform::tray::Event::Minimize => {
					if !quitting {
						self.tray_window.minimize(ctx);
					}
				}
				platform::tray::Event::Unavailable => {
					self.tray_error = Some(if cfg!(target_os = "linux") {
						"Tray unavailable. Start a StatusNotifier host, then toggle the tray off/on."
					} else {
						"Tray unavailable. The window will stay visible."
					});
					self.tray_window.show(ctx);
				}
			}
		}
		self.tray_window.logic(
			ctx,
			self.tray_available(),
			self.window.is_visible().is_some(),
		);
		// The hide command lands after this frame, so the flag leads reported visibility.
		// Occlusion can occur during macOS fullscreen transitions; it is not a request
		// to stop playback (which would also restore the window out of fullscreen).
		let hidden_or_closing = self.tray_window.hidden
			|| ctx.input(|input| {
				input.viewport().minimized == Some(true) || input.viewport().close_requested()
			});
		if self.state.user.is_none()
			|| (!self.state.demo && self.state.auth != AuthState::Authenticated)
			|| hidden_or_closing
		{
			self.audio.stop();
			self.messaging.audio().stop();
			self.video.stop();
			self.messaging.video().stop();
		}
		self.video.poll(self.messaging.video(), ctx);
		let audio = self.audio.poll();
		let player = self.messaging.audio();
		player.position = audio.position.as_secs_f64();
		player.duration = audio.duration.as_secs_f64();
		player.state = match audio.state {
			audio::State::Idle => ui::AudioState::Idle,
			audio::State::Loading => ui::AudioState::Loading,
			audio::State::Playing => ui::AudioState::Playing,
			audio::State::Paused => ui::AudioState::Paused,
			audio::State::Ended => ui::AudioState::Ended,
			audio::State::Failed(error) => ui::AudioState::Failed(error),
		};
		if self.state.auth != AuthState::Authenticated && !self.state.demo {
			self.notifications.clear();
		}
		if let Some(channel) = self.notifications.take_activation()
			&& self.state.auth == AuthState::Authenticated
		{
			self.tray_window.show(ctx);
			if let Some(command) = self.state.select(channel) {
				self.command(command);
			}
		}
		if let Some(alert) = self.notification_runtime.poll(
			&mut self.state,
			&mut self.messaging,
			&self.window,
			ctx,
			self.fixture_only,
		) {
			match alert {
				notification_runtime::Alert::Message {
					channel,
					title,
					body,
					avatar_key,
					image_path,
				} => {
					if self
						.notifications
						.notify_channel(channel, title, body, image_path)
						&& let Some(worker) = &self.avatars
					{
						worker.request(avatar_key);
					}
				}
			}
		}
		self.messaging.voice_ptt_active = self.messaging.voice_push_to_talk
			&& self.state.voice.active.is_some()
			&& (self.messaging.push_to_talk_down(ctx) || self.hotkeys.push_to_talk_down());
	}
	fn ui(&mut self, ui: &mut egui::Ui, _: &mut eframe::Frame) {
		let ctx = ui.ctx().clone();
		self.tray_window.ui(&ctx);
		// Change native hints before drawing, so the clear color and panel alpha
		// agree for the whole frame. OS calls happen only when an effect changes.
		self.sync_window_effects();
		let (close_requested, dropped) = ctx.input_mut(|input| {
			(
				input.viewport().close_requested(),
				std::mem::take(&mut input.raw.dropped_files),
			)
		});
		ui::design::paint_backdrop(&ctx);
		let upload_allowed = self.state.user.is_some()
			&& self.state.gateway_connected
			&& self.state.freshness == model::Freshness::Fresh;
		// A forum container carries its own attachment permission for a post's first message.
		let can_attach = self.state.selected.is_some_and(|channel| {
			self.state.can_attach(channel) || self.state.can_attach_post(channel)
		});
		if let Some(paste) = &self.clipboard
			&& let Some(result) = paste.poll()
		{
			if paste.generation == self.state.generation
				&& Some(paste.channel) == self.state.selected
				&& self.state.user.is_some()
				&& !self.messaging.has_edit_in(self.state.selected)
			{
				match result {
					Ok(clipboard::Content::Text(text)) => {
						self.messaging.pasted_text = Some((paste.channel, paste.target, text));
					}
					Ok(clipboard::Content::File(source)) if upload_allowed && can_attach => {
						if let Err(error) = self.uploads.select_pasted(
							paste.generation,
							paste.channel,
							source,
							self.runtime.handle(),
							&ctx,
						) {
							self.messaging.toasts.push(ui::design::Level::Error, error);
						}
					}
					Ok(clipboard::Content::File(..)) => self.messaging.toasts.push(
						ui::design::Level::Error,
						"Attaching files is unavailable here",
					),
					Err(error) => self.messaging.toasts.push(ui::design::Level::Error, error),
				}
			}
			self.clipboard = None;
		}
		if !can_attach {
			self.uploads.cancel();
		}
		self.uploads.poll(
			self.state.generation,
			self.state.selected,
			self.state.user.is_some() && self.state.gateway_connected,
			&ctx,
		);
		if let Some(command) = self
			.uploads
			.image_send(&mut self.state, self.messaging.image_sharing_enabled)
			&& !self.state.demo
		{
			// Auto-send bypasses the composer, so retain its prepared thumbnail explicitly.
			self.messaging.attachment_previews = self.uploads.previews();
			self.messaging.attachment_files = self.uploads.files();
			self.messaging.stage_pending_upload(&ctx, &command);
			self.command(command);
		}
		// Move native handles once; never load dropped bytes on the rendering thread.
		if !dropped.is_empty() {
			if self.messaging.accepts_server_emoji_drops() {
				let paths: Vec<_> = dropped
					.into_iter()
					.take(11)
					.map(|file| file.path().to_path_buf())
					.collect();
				if !paths.is_empty() {
					self.messaging.queue_server_emoji_drop(paths);
				}
			} else if upload_allowed
				&& can_attach
				&& self.login.is_none()
				&& !self.confirming_close
				&& !self.confirming_logout
				&& !self.messaging.has_edit_in(self.state.selected)
				&& !self.downloads.is_active()
				&& let Some(channel) = self.state.selected
			{
				if let Err(error) = self.uploads.start_drop(
					self.state.generation,
					channel,
					self.runtime.handle(),
					&ctx,
					dropped,
				) {
					self.messaging.toasts.push(ui::design::Level::Error, error);
				}
			} else {
				self.messaging.toasts.push(
					ui::design::Level::Error,
					"File not attached; return to a connected conversation and drop it again",
				);
			}
		}
		// Offline fixtures may stage a synthetic attachment without any upload selection.
		if !self.state.demo || self.uploads.selection().is_some() {
			self.messaging.attachment = self
				.uploads
				.selection()
				.map(|(name, size)| (name.to_owned(), size));
			self.messaging.attachment_previews = self.uploads.previews();
			self.messaging.attachment_files = self.uploads.files();
		}
		self.messaging.upload_busy = self.uploads.busy() || self.clipboard.is_some();
		if let Some(notice) = self.uploads.take_notice() {
			self.messaging.toasts.push(ui::design::Level::Error, notice);
		}
		if !self.state.demo {
			let (progress, sending) = self.uploads.transfer_progress();
			self.messaging.update_upload_progress(progress, sending);
		}
		if self.state.user.is_none() {
			self.downloads.cancel();
		}

		let download_status = match self.downloads.poll() {
			downloads::Status::Idle | downloads::Status::Choosing => String::new(),
			downloads::Status::Downloading { total: 0, .. } => "Loading image…".into(),
			downloads::Status::Downloading { received, total } => {
				format!("Downloading: {} / {} KiB", received / 1024, total / 1024)
			}
			downloads::Status::Saved | downloads::Status::Cancelled | downloads::Status::Copied => {
				String::new()
			}
			downloads::Status::Failed(error) => (*error).into(),
		};
		self.messaging.downloads().active = self.downloads.is_active();
		self.messaging.downloads().status = download_status;
		if close_requested && self.extensions.cleanup_pending() {
			self.extension_close_pending = true;
			ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
		}
		if self.extension_close_pending && !self.extensions.cleanup_pending() {
			self.extension_close_pending = false;
			ctx.send_viewport_cmd(egui::ViewportCommand::Close);
		}
		if close_requested {
			self.downloads.cancel();
		}
		if close_requested && self.downloads.is_active() {
			self.download_close_pending = true;
			ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
		}
		if self.download_close_pending
			&& !self.downloads.is_active()
			&& (!self.confirming_close || self.close_approved)
		{
			self.download_close_pending = false;
			ctx.send_viewport_cmd(egui::ViewportCommand::Close);
		}
		self.poll_avatars(&ctx);
		if let Some((scope, result)) = self.group_icon.poll(&self.state) {
			self.messaging.accept_group_icon(&ctx, scope, result);
		}
		if let Some((scope, result)) = self
			.create_server_icon
			.poll_scoped(self.state.generation, |_| true)
		{
			self.messaging
				.accept_create_server_icon(&ctx, scope, result);
		}
		if let Some((scope, result)) = self
			.profile_avatar
			.poll_scoped(self.state.generation, |user| {
				self.state.user.as_ref().is_some_and(|own| own.id == user)
			}) {
			self.messaging.accept_profile_picture(&ctx, scope, result);
		}
		if let Some((scope, result)) = self.server_icon.poll_server(&self.state) {
			self.messaging.accept_server_icon(&ctx, scope, result);
		}
		if let Some((_, result)) = self.role_icon.poll_scoped(self.state.generation, |guild| {
			self.role_icon_scope.is_some_and(|scope| {
				scope.1 == guild
					&& self.state.server_admin.guild == Some(guild)
					&& self.state.can_edit_role_icon(guild, scope.2)
			})
		}) && let Some(scope) = self.role_icon_scope.take()
		{
			self.messaging.accept_server_role_icon(&ctx, scope, result);
		}

		if let Some((scope, result)) = self.emoji_upload.poll(self.state.generation, |guild| {
			self.state.server_admin.guild == Some(guild) && self.state.can_create_guild_emoji(guild)
		}) {
			self.messaging.accept_server_emojis(&ctx, scope, result);
		}
		if let Some((scope, result)) = self.sticker_upload.poll(self.state.generation, |guild| {
			self.state.server_admin.guild == Some(guild)
				&& self.state.can_create_guild_sticker(guild)
		}) {
			self.messaging.accept_server_sticker(&ctx, scope, result);
		}
		self.messaging.voice_available = true;
		if close_requested
			&& !self.close_approved
			&& ((self.state.demo && self.state.has_unsent())
				|| self
					.state
					.pending
					.iter()
					.any(|p| p.delivery != Delivery::Confirmed)
				|| self.messaging.has_edit()
				|| self.messaging.has_server_settings_changes()
				|| self.messaging.extensions.theme_editor_dirty()
				|| self.state.server_settings.pending
				|| self.state.server_admin.pending
				|| self.uploads.has_unsent()
				|| self.forgetting
				|| self.messaging.startup_busy
				|| self.avatar_cleanup.is_some()
				|| self.cache_pending > 0
				|| self.cache_clears.pending()
				|| (!self.fixture_only && self.app_settings.state.needs_attention())
				|| (!self.fixture_only && self.reading.needs_attention())
				|| (!self.fixture_only && self.game_activity.needs_attention())
				|| (!self.fixture_only && self.tray_setting.needs_attention())
				|| self.cache_error)
		{
			ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
			self.confirming_close = true;
		}
		if self.login.is_some() {
			let p = ui::design::palette(ui);
			egui::Panel::top("login-header")
				.exact_size(platform::LOGIN_HEADER_HEIGHT)
				.show_separator_line(false)
				.frame(
					egui::Frame::NONE
						.fill(ui::design::window_palette(ui).surface)
						.stroke(egui::Stroke::new(1.0, p.border))
						.inner_margin(egui::Margin::symmetric(16, 0)),
				)
				.show(ui, |ui| {
					ui::design::window_drag(ui, ui.max_rect());
					ui.horizontal_centered(|ui| {
						ui.add_space(self.traffic_light_inset());
						let (rect, _) =
							ui.allocate_exact_size(egui::vec2(32.0, 32.0), egui::Sense::hover());
						ui.painter().rect_filled(rect, 8, p.accent);
						ui::icons::paint(
							ui.painter(),
							ui::icons::Icon::Serein,
							rect.shrink(7.0),
							p.accent_text,
						);
						ui.add_space(4.0);
						ui.vertical(|ui| {
							ui.spacing_mut().item_spacing.y = 1.0;
							ui.label(
								ui::design::semibold(ui, "Sign in to Discord", 15.0)
									.color(p.text_strong),
							);
							ui.label(
								egui::RichText::new(
									"discord.com · temporary login window · passwords and 2FA never leave the page",
								)
								.size(12.0)
								.color(p.muted),
							);
						});
						ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
							if !self.messaging.hide_title_bar {
								ui::design::window_controls(ui);
							}
							if ui
								.add(
									egui::Button::new(
										ui::design::medium(ui, "Cancel", 13.0).color(p.text_strong),
									)
									.fill(p.raised)
									.stroke(egui::Stroke::new(1.0, p.border))
									.corner_radius(6)
									.min_size(egui::vec2(0.0, 32.0)),
								)
								.clicked()
							{
								self.login = None;
								self.state.auth = AuthState::Unauthenticated;
							}
						});
					});
				});
			egui::CentralPanel::default()
				.frame(egui::Frame::NONE.fill(p.canvas))
				.show(ui, |ui| {
					ui.centered_and_justified(|ui| {
						ui.label(egui::RichText::new("Loading discord.com…").color(p.muted));
					});
				});
			if let Some(login) = &self.login {
				login.resize(&self.window);
			}
		} else if self.state.user.is_some() {
			self.messaging.storage_status = self.cache_status;
			self.messaging.notification_status = self.notifications.status().label();
			if self.app_settings.state.failed {
				self.messaging.notification_sound_status = "Could not save device notification settings. Changes apply only until restart.";
			}
			let mut commands = self.messaging.show(ui, &mut self.state);
			self.extensions.cancel_stale_message_events(&self.state);
			self.choose_interaction_files(&ctx);
			if let Some(command) = self.captcha.sync(
				&mut self.state,
				&mut self.messaging,
				&self.window,
				&ctx,
				!self.fixture_only && !self.confirming_close && !self.confirming_logout,
			) {
				commands.push(command);
			}
			let player = self.messaging.audio();
			if !player.seen
				|| player.active.is_none()
				|| (!self.state.demo && self.state.auth != AuthState::Authenticated)
			{
				self.audio.stop();
				player.stop();
			}
			if let Some(command) = player.command.take() {
				match command {
					ui::AudioCommand::Play(attachment) => {
						self.audio.volume(player.volume);
						if let Err(error) = self.audio.start(
							attachment,
							self.runtime.handle(),
							&ctx,
							self.fixture_only || self.state.demo,
						) {
							player.state = ui::AudioState::Failed(error);
						}
					}
					ui::AudioCommand::Pause(paused) => self.audio.pause(paused),
					ui::AudioCommand::Seek(seconds) => {
						self.audio.seek(Duration::from_secs_f64(seconds))
					}
					ui::AudioCommand::Volume(volume) => self.audio.volume(volume),
					ui::AudioCommand::Stop => self.audio.stop(),
				}
			}
			let player = self.messaging.video();
			if self.state.demo
				&& let Some(pause) = self.demo_video_autoplay
				&& let Some(message) = self.state.timeline.get(model::Id(601))
				&& let Some(attachment) = message.attachments.first().cloned()
			{
				if player.active.is_none() {
					player.active = Some((message.channel, message.id, attachment.clone()));
					player.state = ui::VideoState::Loading;
					player.seen = true;
					player.command = Some(ui::VideoCommand::Play(attachment));
				} else if player.state == ui::VideoState::Playing && player.position > 1.0 {
					if pause {
						player.command = Some(ui::VideoCommand::Pause(true));
					}
					self.demo_video_autoplay = None;
				}
			}
			if !player.seen
				|| player.active.is_none()
				|| (!self.state.demo && self.state.auth != AuthState::Authenticated)
			{
				self.video.stop();
				player.stop();
			}
			if let Some(command) = player.command.take() {
				self.video.command(
					command,
					player,
					self.runtime.handle(),
					&ctx,
					self.fixture_only || self.state.demo,
				);
			}
			if let Some(fullscreen) = player.take_fullscreen_request() {
				self.window.set_fullscreen(
					fullscreen.then(|| {
						winit::window::Fullscreen::Borderless(self.window.current_monitor())
					}),
				);
			}
			self.notifications.set_enabled(
				self.messaging.notifications_enabled
					&& (!self.fixture_only || self.messaging.notification_test_available),
			);
			if std::mem::take(&mut self.messaging.notification_test_requested)
				&& self.messaging.notification_test_available
			{
				self.notifications.notify();
			}
			// Revalidate scope after navigation without polling workers a second time.
			self.uploads.revalidate_scope(
				self.state.generation,
				self.state.selected,
				self.state.user.is_some() && self.state.gateway_connected,
			);
			if !self.state.selected.is_some_and(|channel| {
				self.state.can_attach(channel) || self.state.can_attach_post(channel)
			}) {
				self.uploads.cancel();
			}
			if let Some(index) = self.messaging.remove_attachment_index.take() {
				self.uploads.remove_at(index);
				if self.state.demo {
					self.messaging.attachment = None;
					self.messaging.attachment_files.clear();
					self.messaging.attachment_previews.clear();
				}
			}
			if std::mem::take(&mut self.messaging.remove_attachment_requested) {
				self.uploads.remove();
				self.messaging.attachment = None;
				self.messaging.attachment_files.clear();
				self.messaging.attachment_previews.clear();
			}
			if std::mem::take(&mut self.messaging.cancel_upload_requested) {
				self.uploads.cancel();
			}
			if let Some(asset) = self.messaging.image_share_requested.take()
				&& self.messaging.image_sharing_enabled
				&& let Some(channel) = self.state.selected
				&& self.state.can_send(channel)
				&& self.state.can_attach(channel)
				&& let Err(error) = self.uploads.start_image_share(
					self.state.generation,
					channel,
					asset,
					self.runtime.handle(),
					&ctx,
					self.state.demo,
				) {
				self.state.status = error;
			}
			if let Some(request) = self.messaging.attachment_paste_requested.take()
				&& let Some(channel) = self.state.selected
			{
				if self.clipboard.is_none() {
					self.clipboard = Some(clipboard::Paste::start(
						self.state.generation,
						channel,
						request,
						self.runtime.handle(),
						&ctx,
					));
				} else {
					self.messaging.toasts.push(
						ui::design::Level::Error,
						"Wait for the current paste to finish",
					);
				}
			}
			if std::mem::take(&mut self.messaging.attach_requested)
				&& let Some(channel) = self.state.selected
				&& (self.state.can_attach(channel) || self.state.can_attach_post(channel))
				&& let Err(error) = self.uploads.start_choose(
					self.state.generation,
					channel,
					self.runtime.handle(),
					&ctx,
					self.window.clone(),
				) {
				self.state.status = error;
			}
			if std::mem::take(&mut self.messaging.downloads().dismiss_requested) {
				self.downloads.dismiss();
			}
			if std::mem::take(&mut self.messaging.downloads().cancel_requested) {
				self.downloads.cancel();
			}
			if let Some((media, copy)) = self.messaging.downloads().embed_request.take()
				&& !self.state.demo
				&& !self.fixture_only
				&& let Err(error) = self.downloads.start_embed(
					media,
					copy,
					self.runtime.handle(),
					&ctx,
					self.window.clone(),
				) {
				self.state.status = error;
			}
			if let Some(attachment) = self.messaging.downloads().copy_request.take()
				&& !self.state.demo
				&& !self.fixture_only
				&& let Err(error) =
					self.downloads
						.start_copy(attachment, self.runtime.handle(), &ctx)
			{
				self.state.status = error;
			}
			if let Some(attachment) = self.messaging.downloads().request.take()
				&& !self.state.demo
				&& !self.fixture_only
				&& let Err(error) = self.downloads.start(
					attachment,
					self.runtime.handle(),
					&ctx,
					self.window.clone(),
				) {
				self.state.status = error;
			}

			if let Some(scope) = self.messaging.take_profile_picture_request() {
				let result = if scope.0 != self.state.generation
					|| !self
						.state
						.user
						.as_ref()
						.is_some_and(|own| own.id == scope.1)
				{
					Err("Sign in before changing your profile picture")
				} else {
					self.profile_avatar.start(
						scope,
						self.runtime.handle(),
						&ctx,
						self.window.clone(),
						"Choose profile picture",
						256,
					)
				};
				if let Err(error) = result {
					self.messaging
						.accept_profile_picture(&ctx, scope, Err(error));
				}
			}
			if let Some(scope) = self.messaging.take_group_icon_request() {
				let result = if scope.0 != self.state.generation || !self.state.is_group_dm(scope.1)
				{
					Err("This group is no longer available")
				} else {
					self.group_icon.start(
						scope,
						self.runtime.handle(),
						&ctx,
						self.window.clone(),
						"Choose group icon",
						256,
					)
				};
				if let Err(error) = result {
					self.messaging.accept_group_icon(&ctx, scope, Err(error));
				}
			}
			if let Some(scope) = self.messaging.take_create_server_icon_request() {
				let result = if scope.0 != self.state.generation || scope.1 != model::Id(0) {
					Err("This server draft is no longer available")
				} else {
					self.create_server_icon.start(
						scope,
						self.runtime.handle(),
						&ctx,
						self.window.clone(),
						"Choose server icon",
						256,
					)
				};
				if let Err(error) = result {
					self.messaging
						.accept_create_server_icon(&ctx, scope, Err(error));
				}
			}
			if let Some(scope) = self.messaging.take_server_role_icon_request() {
				let result = if scope.0 != self.state.generation
					|| !self.state.can_edit_role_icon(scope.1, scope.2)
				{
					Err("You can no longer change this role icon")
				} else {
					self.role_icon.start(
						(scope.0, scope.1, scope.3),
						self.runtime.handle(),
						&ctx,
						self.window.clone(),
						"Choose role icon",
						128,
					)
				};
				match result {
					Ok(()) => self.role_icon_scope = Some(scope),
					Err(error) => self
						.messaging
						.accept_server_role_icon(&ctx, scope, Err(error)),
				}
			}
			if let Some(scope) = self.messaging.take_server_icon_request() {
				let result =
					if scope.0 != self.state.generation || !self.state.can_manage_guild(scope.1) {
						Err("You can no longer manage this server")
					} else {
						self.server_icon.start(
							scope,
							self.runtime.handle(),
							&ctx,
							self.window.clone(),
							"Choose server icon",
							512,
						)
					};
				if let Err(error) = result {
					self.messaging.accept_server_icon(&ctx, scope, Err(error));
				}
			}
			if let Some((generation, guild, request, paths)) =
				self.messaging.take_server_emoji_request()
			{
				let scope = (generation, guild, request);
				let result = if generation != self.state.generation
					|| !self.state.can_create_guild_emoji(guild)
				{
					Err("You can no longer upload emoji to this server")
				} else {
					self.emoji_upload.start(
						scope,
						paths,
						self.runtime.handle(),
						&ctx,
						self.window.clone(),
					)
				};
				if let Err(error) = result {
					self.messaging.accept_server_emojis(&ctx, scope, Err(error));
				}
			}
			if let Some((generation, guild, request)) = self.messaging.take_server_sticker_request()
			{
				let scope = (generation, guild, request);
				let result = if generation != self.state.generation
					|| !self.state.can_create_guild_sticker(guild)
				{
					Err("You can no longer upload stickers to this server")
				} else {
					self.sticker_upload.start(
						scope,
						self.runtime.handle(),
						&ctx,
						self.window.clone(),
					)
				};
				if let Err(error) = result {
					self.messaging
						.accept_server_sticker(&ctx, scope, Err(error));
				}
			}
			for key in self.messaging.take_avatar_requests() {
				if !self
					.avatars
					.as_ref()
					.is_some_and(|worker| worker.request(key.clone()))
				{
					self.messaging.accept_avatar(&ctx, key, None);
				}
			}
			if self.messaging.reconnect_requested {
				if let Some(store) = &mut self.store {
					store.cancel_load();
				}
				self.messaging.reconnect_requested = false;
				let wake = ctx.clone();
				match platform::LoginView::open(self.window.clone(), move || wake.request_repaint())
				{
					Ok(login) => self.login = Some(login),
					Err(_) => self.state.status = "Platform login webview unavailable",
				}
			}
			let draft_changes = std::mem::take(&mut self.messaging.draft_changes);
			for channel in draft_changes {
				let content = recovery_draft(&self.state, channel);
				self.queue_cache(cache::Operation::SaveDraft { channel, content });
			}
			if self.messaging.clear_cache_requested {
				self.messaging.clear_cache_requested = false;
				self.state.clear_cached_history();
				self.clear_avatars(&ctx);
				self.queue_cache(cache::Operation::ClearHistory);
			}
			for command in commands {
				self.command(command);
			}
			self.poll_voice(&ctx);
			if self.messaging.logout_requested {
				self.messaging.logout_requested = false;
				self.request_session_end(&ctx, SessionEnd::Logout);
			}
		} else if let Some(stage) = self.restoring() {
			self.restoring_screen(ui, stage);
		} else {
			self.sign_in_screen(ui);
		}
		#[cfg(feature = "demo")]
		if let Some(diagnostic) = &self.rendering_demo {
			diagnostic.show(&ctx, &self.window);
		}
		let appearance = ctx.options(|options| options.theme_preference);
		#[cfg(target_os = "linux")]
		if self.window.is_decorated() == self.messaging.hide_window_decorations {
			self.window
				.set_decorations(!self.messaging.hide_window_decorations);
		}
		#[cfg(target_os = "windows")]
		if self.window.is_decorated() != self.messaging.hide_title_bar {
			self.window.set_decorations(self.messaging.hide_title_bar);
			align_undecorated_surface(&self.window);
		}
		#[cfg(target_os = "macos")]
		if let Err(error) =
			platform::window::set_native_title_bar(&self.window, self.messaging.hide_title_bar)
		{
			self.messaging.hide_title_bar = false;
			self.state.status = error;
		}
		self.sync_customization(&ctx);
		self.save_app_preferences();
		self.messaging.updates_save_failed = self.app_settings.state.failed;
		self.save_reading_preferences(&ctx);
		self.sync_own_presence(&ctx);
		self.sync_game_activity(&ctx);
		self.startup.sync(
			&ctx,
			&self.runtime,
			&mut self.messaging,
			self.fixture_only || self.state.demo,
		);
		self.sync_tray(&ctx);
		if appearance != self.appearance {
			self.appearance = appearance;
			self.appearance_changed = true;
			let preference = match appearance {
				egui::ThemePreference::System => local_store::Appearance::System,
				egui::ThemePreference::Light => local_store::Appearance::Light,
				egui::ThemePreference::Dark => local_store::Appearance::Dark,
			};
			self.queue_cache_for(model::Id(0), cache::Operation::SaveAppearance(preference));
		}
		if std::mem::take(&mut self.state.gifs.favorites_changed) {
			let favorites = self.state.gifs.favorites.clone();
			self.queue_cache(cache::Operation::SaveGifFavorites(favorites));
		}
		if !self.fixture_only
			&& !self.state.demo
			&& let Some(user) = &self.state.user
		{
			self.cache_pending += usize::from(queue_channel_preferences(
				self.cache.as_ref(),
				&mut self.messaging,
				self.state.generation,
				user.id,
			));
		}
		if self.messaging.channel_preferences_changed
			&& !self.messaging.channel_preferences_save_pending
		{
			self.messaging.channel_preferences_changed = false;
			if !self.state.demo && !self.fixture_only {
				self.messaging.channel_preferences_save_pending =
					self.queue_cache(cache::Operation::SaveChannelPreferences(
						self.messaging.channel_preferences.clone(),
					));
				self.messaging.channel_preferences_status =
					if self.messaging.channel_preferences_save_pending {
						""
					} else {
						"Could not save channel preferences."
					};
			}
		}
		if let Some(variant) = self.messaging.theme_variant_changed.take() {
			self.variant_changed = true;
			let key = (variant != ui::design::Variant::Standard).then(|| variant.key().to_owned());
			self.queue_cache_for(model::Id(0), cache::Operation::SaveThemeVariant(key));
		}
		let switch = self.messaging.switch_account_requested.take();
		let add = std::mem::take(&mut self.messaging.add_account_requested);
		let forget = self.messaging.forget_account_requested.take();
		// Fixture builds render the switcher for captures but never end their offline session.
		if !self.fixture_only {
			if let Some(account) = switch
				&& self
					.state
					.user
					.as_ref()
					.is_none_or(|user| user.id != account)
			{
				self.request_session_end(&ctx, SessionEnd::Switch(account));
			}
			if add {
				self.request_session_end(&ctx, SessionEnd::Add);
			}
			if let Some(account) = forget {
				self.confirming_forget = Some(account);
			}
		}
		self.sync_account_roster();
		self.confirm_forget_dialog(&ctx);
		if self.confirming_close || self.confirming_logout {
			let mut notes: Vec<&str> = Vec::new();
			if self.messaging.extensions.theme_editor_dirty() {
				notes.push("Unsaved theme changes will be discarded.");
			}
			if self.forgetting {
				notes.push("Wait for saved-login removal to finish.");
			}
			if self.extensions.cleanup_pending() {
				notes.push("Removing extension data before closing.");
			}
			if self.cache_clears.pending() {
				notes.push(
					"Cached history cleanup is pending; closing now may leave deleted messages on disk.",
				);
			}
			if !self.fixture_only && self.app_settings.state.needs_attention() {
				notes.push(self.app_settings.state.status());
			}
			if !self.fixture_only && self.reading.needs_attention() {
				notes.push(self.reading.status());
			}
			if !self.fixture_only && self.game_activity.needs_attention() {
				notes.push(self.game_activity.status());
			}
			if !self.fixture_only && self.tray_setting.needs_attention() {
				notes.push(self.tray_setting.status());
			}
			let mut confirm = ui::dialog::Confirm::new(
				"leave-session",
				if self.confirming_logout && !self.end_intent.forgets() {
					"Leave this account?"
				} else {
					"Leave this session?"
				},
				if self.confirming_logout && !self.end_intent.forgets() {
					"Saved text drafts survive the switch; selected files must be reselected. This account stays in the switcher."
				} else {
					"Saved text drafts survive exit; selected files must be reselected. Logging out removes local account data."
				},
			)
			.danger()
			.confirm_label("Discard and Continue")
			.cancel_label("Keep Working")
			.enabled(!self.forgetting);
			if !notes.is_empty() {
				confirm = confirm.note(ui::dialog::Level::Warning, notes.join("\n"));
			}
			match confirm.show(&ctx) {
				Some(ui::dialog::Choice::Confirmed) => {
					if self.confirming_close {
						self.close_approved = true;
						self.uploads.cancel();
						if self.downloads.is_active() {
							self.downloads.cancel();
							self.download_close_pending = true;
						} else {
							ctx.send_viewport_cmd(egui::ViewportCommand::Close);
						}
					} else {
						let intent = self.end_intent;
						self.finish_session_end(&ctx, intent);
					}
				}
				Some(ui::dialog::Choice::Cancelled) => {
					self.updater.cancel_restart();
					self.confirming_close = false;
					self.tray_window.cancel_quit();
					self.extension_close_pending = false;
					self.confirming_logout = false;
					self.end_intent = SessionEnd::Logout;
					self.download_close_pending = false;
				}
				None => {}
			}
		}
		ui::design::window_resize(&ctx);
		self.frame_metrics.reflows = self.messaging.timeline_reflows();
		self.frame_metrics.finish();
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn user_action_results_use_toasts() {
		let success = Event::UserAction(client_core::user_actions::Event::Written {
			action: client_core::user_actions::Action::Nickname {
				user: model::Id(2),
				text: "Synthetic".into(),
			},
			request: 1,
			result: Ok(()),
		});
		let (level, text) = user_action_notice(&success).unwrap();
		assert!(matches!(level, ui::design::Level::Success));
		assert_eq!(text, "Nickname saved");
	}
	#[test]
	fn frame_sample_is_bounded_demo_only_and_excludes_warmup() {
		for (demo, value) in [
			(false, "=8,15"),
			(true, "=0,15"),
			(true, "=8,0"),
			(true, "=601,15"),
			(true, "=8,601"),
			(true, "=8,15,1"),
			(true, "=8.5,15"),
			(true, ""),
		] {
			assert!(parse_frame_sample(demo, value).is_err());
		}
		let durations = parse_frame_sample(true, "=8,15").unwrap();
		let mut metrics = FrameMetrics::new(Some(durations));
		let ready = metrics.sample.as_ref().unwrap().ready;
		assert!(!metrics.sample_active_at(ready - Duration::from_secs(1)));
		assert_eq!(metrics.frames, 0);
		assert!(metrics.sample_active_at(ready));
		assert!(metrics.sample_active_at(ready + Duration::from_secs(14)));
		assert!(!metrics.sample_active_at(ready + Duration::from_secs(15)));
		assert!(!metrics.sample_active_at(ready + Duration::from_secs(16)));
		assert!(metrics.sample.as_ref().unwrap().complete);
	}
	#[test]
	fn disk_cache_does_not_replace_resident_previews_or_deleted_positions() {
		for deleted in [false, true] {
			let mut state = test_support::demo_state();
			let alpha = state.selected.unwrap();
			let beta = model::Id(900);
			let mut other = state
				.channels
				.iter()
				.find(|c| c.id == alpha)
				.unwrap()
				.clone();
			other.id = beta;
			other.guild = None;
			other.kind = 1;
			state.channels.push(other);
			state.timeline.clear();
			state
				.timeline
				.insert(test_support::message(1001, alpha), false, false)
				.unwrap();
			if deleted {
				state.timeline.delete(model::Id(1001)).unwrap();
			}
			state.select(beta).unwrap();
			state.apply(Envelope {
				generation: state.generation,
				event: Event::History {
					channel: beta,
					request: state.request,
					older: false,
					messages: vec![test_support::message(1002, beta)],
				},
			});
			state.select(alpha).unwrap();
			let request = state.request;
			assert_eq!(state.timeline.row_count(), 1);
			assert!(!wants_cached_history(&state, alpha, request));
			hydrate_cached_history(
				&mut state,
				alpha,
				request,
				vec![test_support::message(1003, alpha)],
			);
			assert_eq!(
				state.timeline.row_ids().collect::<Vec<_>>(),
				[model::Id(1001)]
			);
			assert_eq!(state.timeline.is_empty(), deleted);
			assert_eq!(state.freshness, model::Freshness::Loading);
			state.clear_cached_history();
			assert_eq!(
				state.timeline.row_count(),
				1,
				"Clear keeps the displayed conversation"
			);
			state.select(beta).unwrap();
			assert_eq!(
				state.timeline.row_count(),
				0,
				"Cleared dormant windows cannot return"
			);
			assert!(wants_cached_history(&state, beta, state.request));
		}
	}
	#[test]
	fn delayed_cache_results_cannot_hydrate_after_a_known_deletion() {
		let mut state = test_support::demo_state();
		let channel = state.selected.unwrap();
		state.timeline.clear();
		state.history(None);
		let request = state.request;
		let safety = cache::HistorySafety::default();
		let outcome = |epoch| cache::Outcome::Channel {
			channel,
			request,
			epoch,
			messages: vec![test_support::message(1000, channel)],
		};
		let old_epoch = safety.epoch();
		safety.invalidate();
		hydrate_cache_result(&mut state, &safety, outcome(old_epoch));
		assert!(state.timeline.is_empty());
		safety.block();
		hydrate_cache_result(&mut state, &safety, outcome(safety.epoch()));
		assert!(state.timeline.is_empty());
		safety.cleared();
		hydrate_cache_result(&mut state, &safety, outcome(safety.epoch()));
		assert_eq!(state.timeline.len(), 1);
	}

	#[test]
	fn cached_history_requires_current_readable_navigation_and_pending_request() {
		let mut state = test_support::demo_state();
		let channel = state.selected.unwrap();
		state.timeline.clear();
		state.history(None);
		let request = state.request;
		hydrate_cached_history(
			&mut state,
			channel,
			request,
			vec![test_support::message(1000, channel)],
		);
		assert_eq!(state.timeline.len(), 1);
		assert_eq!(state.freshness, model::Freshness::Loading);
		state.timeline.clear();
		let permissions = state.permissions.clone();
		state.permissions.channels.remove(&channel);
		state.permissions.clear_cache();
		assert!(!state.can_read_history(channel));
		hydrate_cached_history(
			&mut state,
			channel,
			request,
			vec![test_support::message(1000, channel)],
		);
		assert!(
			state.timeline.is_empty(),
			"Loaded navigation alone does not authorize cached history"
		);
		state.permissions = permissions;
		for invalid in [
			vec![test_support::message(1001, model::Id(999))],
			vec![test_support::message(1002, channel)],
		] {
			hydrate_cached_history(&mut state, channel, request.wrapping_sub(1), invalid);
			assert!(state.timeline.is_empty());
		}
		hydrate_cached_history(
			&mut state,
			channel,
			request,
			vec![test_support::message(1001, model::Id(999))],
		);
		assert!(state.timeline.is_empty());
		state.history_pending = false;
		hydrate_cached_history(
			&mut state,
			channel,
			request,
			vec![test_support::message(1000, channel)],
		);
		assert!(state.timeline.is_empty());
		state.history_pending = true;
		let mut channels = state.channels.clone();
		channels.retain(|c| c.id != channel);
		state.apply(Envelope {
			generation: state.generation,
			event: Event::Ready {
				user: state.user.clone().unwrap(),
				guilds: state.guilds.clone(),
				permissions: test_support::permission_snapshot(&state),
				channels,
			},
		});
		hydrate_cached_history(
			&mut state,
			channel,
			request,
			vec![test_support::message(1000, channel)],
		);
		assert!(state.timeline.is_empty());
		// Even an inconsistent queued-cache admission state cannot bypass current navigation.
		state.selected = Some(channel);
		state.request = request;
		state.freshness = model::Freshness::Loading;
		state.history_pending = true;
		hydrate_cached_history(
			&mut state,
			channel,
			request,
			vec![test_support::message(1000, channel)],
		);
		assert!(state.timeline.is_empty());
		let mut restored = test_support::demo_state()
			.channels
			.into_iter()
			.find(|c| c.id == channel)
			.unwrap();
		restored.kind = 4; // Categories do not support text; voice channels do.
		state.channels.push(restored);
		hydrate_cached_history(
			&mut state,
			channel,
			request,
			vec![test_support::message(1000, channel)],
		);
		assert!(state.timeline.is_empty());
	}
	#[test]
	fn correlated_confirmation_preserves_other_pending_text_and_ignores_unrelated_events() {
		let mut state = test_support::demo_state();
		let channel = state.selected.unwrap();
		state.drafts.insert(channel, "first pending draft".into());
		let Command::Send { nonce, .. } = state.prepare_send().unwrap() else {
			panic!()
		};
		state.drafts.insert(channel, "second pending draft".into());
		state.prepare_send().unwrap();
		let mut message = test_support::message(1000, channel);
		message.author = state.user.clone().unwrap();
		message.nonce = Some(nonce.clone());
		let mut foreign = message.clone();
		foreign.author.id = model::Id(999);
		assert!(confirmed_recovery_channel(&state, &Event::Message(foreign)).is_none());
		let confirmation = Event::SendResult {
			nonce,
			result: Ok(message),
		};
		assert_eq!(
			confirmed_recovery_channel(&state, &confirmation),
			Some(channel)
		);
		state.apply(Envelope {
			generation: state.generation,
			event: confirmation,
		});
		assert_eq!(recovery_draft(&state, channel), "second pending draft");
		state.drafts.insert(channel, String::new());
		assert_eq!(recovery_draft(&state, channel), "second pending draft");
		state.drafts.insert(channel, "new unsent edit".into());
		assert_eq!(recovery_draft(&state, channel), "new unsent edit");
		assert!(!changes_active_history(
			&state,
			&Event::Message(test_support::message(1001, model::Id(999)))
		));
		assert!(!changes_active_history(
			&state,
			&Event::History {
				channel,
				request: state.request.wrapping_sub(1),
				older: false,
				messages: vec![]
			}
		));
		assert!(changes_active_history(
			&state,
			&Event::DeleteBulk {
				channel,
				ids: vec![model::Id(1000)]
			}
		));
	}
}
