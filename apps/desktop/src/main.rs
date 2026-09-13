#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]
mod app_settings;
mod audio;
mod avatars;
mod cache;
mod captcha;
#[cfg(feature = "demo")]
mod channel_demo;
mod clipboard;
mod connection;
mod credentials;
mod downloads;
mod emoji_upload;
mod extension_bridge;
mod extensions;
mod game_activity;
mod group_icon;
mod notification_runtime;
mod notification_sounds;
mod reading_settings;
mod screen;
#[cfg(feature = "demo")]
mod server_settings_demo;
mod startup;
mod toggle_setting;
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
#[cfg(feature = "developer-session")]
use zeroize::Zeroizing;

fn main() -> eframe::Result {
	let demo = std::env::args().any(|arg| arg == "--demo");
	let start_minimized = startup::minimized_launch(demo, std::env::args());
	if !cfg!(feature = "demo")
		&& std::env::args().any(|arg| arg == "--demo" || arg.starts_with("--demo-"))
	{
		eprintln!("Demo support is not included; rebuild with --features demo and run with --demo");
		std::process::exit(2);
	}
	#[cfg(feature = "demo")]
	if demo && std::env::args().any(|arg| arg == "--demo-check-extensions") {
		demo_check_extensions();
		return Ok(());
	}
	#[cfg(target_os = "windows")]
	let icon = include_bytes!("../../../packaging/windows/serein.png").as_slice();
	#[cfg(target_os = "linux")]
	let icon = include_bytes!("../../../packaging/linux/hicolor/256x256/apps/serein.png").as_slice();
	let options = eframe::NativeOptions {
		viewport: {
			let builder = egui::ViewportBuilder::default()
				.with_inner_size([1120.0, 760.0])
				.with_min_inner_size([760.0, 520.0])
				.with_active(!start_minimized)
				.with_app_id("org.serein.desktop");
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
			} else {
				builder
			}
		},
		renderer: eframe::Renderer::Wgpu,
		persist_window: false,
		persistence_path: None,
		..Default::default()
	};
	eframe::run_native(
		"Serein",
		options,
		Box::new(move |cc| {
			let desktop = Desktop::new(cc, demo)?;
			if start_minimized {
				cc.egui_ctx
					.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
			}
			Ok(Box::new(desktop))
		}),
	)
}
/// One offline debug path through the shipped Wasm, reducer, and native egui rows.
#[cfg(feature = "demo")]
fn demo_check_extensions() {
	let enabled =
		extensions::demo_check_examples().expect("starter packages activate with consent");
	let mut state = test_support::demo_state();
	let channel = state.selected.expect("demo conversation");
	state.timeline.clear();
	state.set_preserve_deleted_messages(enabled);
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
	for _ in 0..3 {
		let frame = ctx.run_ui(
			egui::RawInput {
				screen_rect: Some(egui::Rect::from_min_size(
					egui::Pos2::ZERO,
					egui::vec2(1120.0, 760.0),
				)),
				..Default::default()
			},
			|ui| {
				let _ = messaging.show(ui, &mut state);
			},
		);
		saw_deleted |= frame.shapes.iter().any(|shape| matches!(&shape.shape, egui::Shape::Text(text) if text.galley.text().contains("Deleted - kept by Message delete protector")));
		frame.drop_without_applying_deltas();
	}
	assert!(
		saw_deleted,
		"native timeline renders the retained deletion label"
	);
	state.set_preserve_deleted_messages(false);
	assert!(
		state.timeline.get_display(message.id).is_none(),
		"disabling releases preserved text"
	);
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
		"default deletion still removes the payload"
	);
	println!(
		"Extension debug check passed: one Wasm protector, five themes, consent, red deleted row, stale-history rejection and disable cleanup."
	);
}

/// Opt-in aggregate CPU callback timings; no payloads, per-frame logs, or repaint timer.
struct FrameMetrics {
	enabled: bool,
	started: Option<std::time::Instant>,
	frames: u64,
	inputless: u64,
	buckets: [u64; 8],
	reflows: (u64, u64),
}
impl Default for FrameMetrics {
	fn default() -> Self {
		Self {
			enabled: std::env::var_os("SEREIN_FRAME_DIAGNOSTICS").is_some_and(|v| v == "1"),
			started: None,
			frames: 0,
			inputless: 0,
			buckets: [0; 8],
			reflows: (0, 0),
		}
	}
}
impl FrameMetrics {
	fn begin(&mut self, ctx: &egui::Context) {
		if self.enabled {
			self.started = Some(std::time::Instant::now());
			self.inputless += u64::from(ctx.input(|i| i.events.is_empty()));
		}
	}
	fn finish(&mut self) {
		if let Some(started) = self.started.take() {
			let micros = started.elapsed().as_micros();
			let bucket = [1000, 2000, 4000, 8000, 16000, 32000, 64000]
				.partition_point(|limit| *limit < micros);
			self.buckets[bucket] += 1;
			self.frames += 1;
		}
	}
}
impl Drop for FrameMetrics {
	fn drop(&mut self) {
		if self.enabled {
			use std::io::Write;
			let _ = writeln!(
				std::io::stderr(),
				"[Serein frames] callbacks={} without_input={} cpu_us_buckets(1000,2000,4000,8000,16000,32000,64000,above)={:?} reflows(total,consecutive)={:?}",
				self.frames,
				self.inputless,
				self.buckets,
				self.reflows
			);
		}
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
	downloads: downloads::Downloads,
	audio: audio::Audio,
	video: video::Video,
	/// Offline fixture flags start (and optionally pause) the demo attachment without input.
	demo_video_autoplay: Option<bool>,
	notifications: platform::notifications::Notifications,
	notification_runtime: notification_runtime::Runtime,
	uploads: uploads::Uploads,
	group_icon: group_icon::GroupIcon,
	server_icon: group_icon::GroupIcon,
	role_icon: group_icon::GroupIcon,
	role_icon_scope: Option<(u64, model::Id, model::Id, u64)>,
	emoji_upload: emoji_upload::EmojiUpload,
	clipboard: Option<clipboard::Paste>,
	download_close_pending: bool,
	window: Arc<winit::window::Window>,
	monitor_geometry: Option<(Option<egui::Rect>, Option<f32>)>,
	monitor_period: Option<Duration>,
	frame_metrics: FrameMetrics,
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
	reading: reading_settings::ReadingSettings,
	app_settings: app_settings::Settings,
	game_activity: toggle_setting::Settings,
	tray_setting: toggle_setting::Settings,
	startup: startup::Startup,
	tray: Option<platform::tray::Tray>,
	tray_error: Option<&'static str>,
	/// `--demo-reply`: keeps two synthetic typists active on the selected fixture channel.
	#[cfg(feature = "demo")]
	demo_typing: bool,
	variant_changed: bool,
	pending_save: Option<Arc<SessionSecret>>,
	credential_status: &'static str,
	forgetting: bool,
	confirming_close: bool,
	confirming_logout: bool,
	close_approved: bool,
	fixture_only: bool,
	authorized: bool,
	#[cfg(feature = "demo")]
	synthetic_id: u64,
	#[cfg(feature = "developer-session")]
	token_input: Zeroizing<String>,
}
/// Check only navigation whose effective access can change with this event.
fn access_candidates(state: &State, event: &Event) -> Vec<model::Id> {
	use client_core::permissions::Event as Permission;
	let (guilds, channel): (Option<Vec<model::Id>>, Option<model::Id>) = match event {
		Event::Ready { .. } | Event::Permissions(Permission::Snapshot(_)) => (None, None),
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
	let alpha = if ui.visuals().dark_mode { 0.16 } else { 0.10 };
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
			activities: vec![model::RichActivity {
				kind: 0,
				name: "Stardew Valley".into(),
				details: Some("Tending the synthetic farm".into()),
				state: Some("Spring - Day 12".into()),
				image: Some(model::ActivityImage::Asset {
					application: model::Id(9001),
					asset: model::Id(9002),
				}),
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
		rows: members.into_iter().map(Some).collect(),
		freshness: model::Freshness::Fresh,
	}
}

impl Desktop {
	fn new(
		cc: &eframe::CreationContext<'_>,
		demo: bool,
	) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
		ui::fonts::install(&cc.egui_ctx);
		ui::emoji::install_async(&cc.egui_ctx)?;
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
				if std::env::args().any(|arg| arg == "--demo-forwarded") {
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
				} else if std::env::args().any(|arg| arg == "--demo-chat") {
					test_support::chat_demo_state()
				} else {
					test_support::demo_state()
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
			let fixture = demo_members(None, model::Id(22), 0);
			state.direct_presences = fixture
				.rows
				.into_iter()
				.flatten()
				.filter(|member| member.user.id != model::Id(1))
				.map(|member| model::MemberPresence {
					user: member.user.id,
					status: member.status,
					custom_status: member.custom_status,
					activities: member.activities,
				})
				.collect();
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
		let mut tray_setting = toggle_setting::Settings::default();
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
		messaging.tray_available = platform::tray::supported();
		let startup = startup::Startup::new(&cc.egui_ctx, &runtime, &mut messaging, demo);
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
			state.reply = state.timeline.iter().last().map(|message| message.id);
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
			messaging.preview_account_menu(state.generation);
			state.status = "Offline fixture · account popout opened at startup";
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
		if demo && std::env::args().any(|arg| arg == "--demo-login") {
			// Fixture-only: render the sign-in screen without a session.
			state.user = None;
			state.status = "Disconnected";
		}
		#[cfg(feature = "demo")]
		if demo && std::env::args().any(|arg| arg == "--demo-server-settings") {
			server_settings_demo::open(&mut state, &mut messaging);
		}
		Ok(Self {
			extensions: extension_bridge::Bridge::default(),
			extension_close_pending: false,
			login: None,
			captcha: captcha::Captcha::default(),
			connection: None,
			state,
			messaging,
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
				platform::notifications::Notifications::new(move || wake.request_repaint())
			},
			uploads: uploads::Uploads::default(),
			group_icon: group_icon::GroupIcon::default(),
			server_icon: group_icon::GroupIcon::default(),
			role_icon: group_icon::GroupIcon::default(),
			role_icon_scope: None,
			emoji_upload: emoji_upload::EmojiUpload::default(),
			clipboard: None,
			download_close_pending: false,
			window: cc
				.winit_window()
				.ok_or("Native window unavailable")?
				.clone(),
			monitor_geometry: None,
			monitor_period: None,
			frame_metrics: FrameMetrics::default(),
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
			reading,
			app_settings,
			game_activity,
			tray_setting,
			startup,
			tray: None,
			tray_error: None,
			#[cfg(feature = "demo")]
			demo_typing,
			variant_changed: false,
			pending_save: None,
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
			close_approved: false,
			fixture_only: demo,
			authorized: false,
			#[cfg(feature = "demo")]
			synthetic_id,
			#[cfg(feature = "developer-session")]
			token_input: Zeroizing::new(String::new()),
		})
	}
	fn connect(&mut self, secret: SessionSecret, save: bool, ctx: &egui::Context) {
		self.role_icon.cancel();
		self.role_icon_scope = None;
		self.group_icon.cancel();
		self.server_icon.cancel();
		self.emoji_upload.cancel();
		if let Some(store) = &mut self.store {
			store.cancel_load();
		}
		self.credential_status = if save {
			"Login will be saved after Discord connects"
		} else {
			"Connecting with the supplied session; saved login unchanged"
		};
		self.voice.stop();
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
		self.messaging.channel_preferences_reload = false;
		self.messaging.channel_preferences_save_pending = false;
		self.messaging.channel_preferences_status = "";
		self.messaging.own_presence = model::OwnPresence::default();
		self.messaging.own_presence_changed = false;
		self.messaging.draft_restore_pending = false;
		self.state.auth = AuthState::Authenticating;
		self.state.status = "Connecting to Discord…";
		let secret = Arc::new(secret);
		self.pending_save = save.then(|| secret.clone());
		self.connection = Some(connection::Connection::start(
			self.runtime.handle(),
			secret,
			self.state.generation,
			self.state.user.as_ref().map(|u| u.id),
			ctx.clone(),
		));
	}
	fn logout(&mut self, ctx: &egui::Context) {
		self.captcha.close();
		self.notification_runtime.clear(&self.window);
		let extension_logout = self.extensions.logout(ctx);
		self.role_icon.cancel();
		self.role_icon_scope = None;
		self.group_icon.cancel();
		self.server_icon.cancel();
		self.emoji_upload.cancel();
		self.notifications.clear();
		self.uploads.cancel();
		if let Some(store) = &mut self.store {
			store.cancel_load();
		}
		self.downloads.cancel();
		self.audio.stop();
		self.video.stop();
		self.voice.stop();
		let was_demo = self.state.demo;
		self.clear_avatars(ctx);
		self.login = None;
		self.connection = None;
		self.pending_save = None;
		let old_account = self.state.user.as_ref().filter(|_| !was_demo).map(|u| u.id);
		self.state.logout();
		if let (Some(cache), Some(account)) = (&self.cache, old_account) {
			if cache.queue(self.state.generation, account, cache::Operation::Forget) {
				self.cache_pending += 1;
			} else {
				self.cache_error = true;
				self.cache_status = "Could not queue local account data removal";
			}
		}
		self.messaging.clear();
		if let Err(error) = extension_logout {
			self.messaging.extensions.status = error;
			self.cache_error = true;
			self.cache_status = "Extension account data removal could not be queued";
		}
		self.app_settings.apply(&mut self.messaging);
		self.messaging.share_game_activity = self.game_activity.enabled;
		ctx.memory_mut(|m| *m = egui::Memory::default());
		ui::design::apply(ctx);
		ctx.set_theme(self.appearance);
		self.messaging
			.apply_reading_preferences(ctx, self.reading.current);
		ctx.clear_animations();
		#[cfg(feature = "developer-session")]
		{
			self.token_input = Zeroizing::new(String::new());
		}
		if !was_demo && let Some(store) = &self.store {
			self.forgetting = store
				.send
				.try_send((self.state.generation, credentials::Operation::Forget))
				.is_ok();
			self.credential_status = if self.forgetting {
				"Removing saved login…"
			} else {
				"Credential queue unavailable; saved login may remain"
			};
		}
		self.confirming_logout = false;
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
			match platform::tray::Tray::new(self.window.clone(), move || wake.request_repaint()) {
				Ok(tray) => self.tray = Some(tray),
				Err(error) => self.tray_error = Some(error),
			}
		}
		self.messaging.tray_status = self
			.tray_error
			.unwrap_or_else(|| self.tray_setting.status());
		if previous_status != self.messaging.tray_status {
			ctx.request_repaint();
		}
	}
	fn sync_own_presence(&mut self, ctx: &egui::Context) {
		let changed = std::mem::take(&mut self.messaging.own_presence_changed);
		let previous_status = self.messaging.own_presence_status;
		self.messaging.own_presence_status = if !self.messaging.own_presence.valid() {
			"Status must be at most 128 characters without line breaks or surrounding spaces."
		} else if self.state.demo || self.fixture_only {
			"Offline preview: not shared or saved."
		} else if let Some(connection) = &self.connection {
			if connection.own_presence.is_closed()
				|| (changed
					&& connection
						.own_presence
						.send(self.messaging.own_presence.clone())
						.is_err())
			{
				"Could not update status: connection unavailable."
			} else if !self.state.gateway_connected {
				"Waiting for connection; public visibility unconfirmed."
			} else {
				"This session only; public visibility unconfirmed."
			}
		} else {
			"Not connected; status is not shared."
		};
		if changed || previous_status != self.messaging.own_presence_status {
			ctx.request_repaint();
		}
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
			if self.game_activity.enabled && self.state.gateway_connected {
				match &*connection.game_activity.borrow() {
					Ok(game) => {
						self.messaging.own_game = game.as_ref().map(model::RichActivity::summary);
						own_activity = game.clone();
						if game.is_some() && !self.game_activity.needs_attention() {
							use discord_gateway::ActivityObservation as Observation;
							self.messaging.game_activity_status =
								match *connection.activity_observation.borrow() {
									Observation::Unconfirmed => {
										"Game detected. Waiting for Discord to confirm."
									}
									Observation::ServerReceived => {
										"Discord received your game. Public sharing is not confirmed."
									}
									Observation::ServerListed => {
										"Discord lists your game. Server and friend privacy settings still apply."
									}
									Observation::ServerHidden => {
										self.messaging.discord_activity_sharing_retry = true;
										"Discord is hiding your game. Check Activity Privacy in Discord."
									}
									Observation::ServerMissing => {
										"Discord is not listing your game. Sharing is not confirmed."
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
	fn command(&mut self, command: Command) {
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
				// Demo preference changes are applied synchronously by client-core.
				Command::AccountNotificationSettings { .. }
				| Command::MessagingPermissions { .. } => return,
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
				Command::GuildFolders(settings) => {
					Event::GuildFolders(Ok(settings.unwrap_or_default()))
				}
				Command::SendServerInvite {
					guild,
					user,
					code,
					nonce,
					request,
				} => {
					let recipient = self
						.state
						.friends()
						.find(|f| f.id == user)
						.cloned()
						.unwrap();
					let channel = self
						.state
						.channels
						.iter()
						.find(|c| {
							c.kind == 1
								&& c.guild.is_none() && c.recipients.len() == 1
								&& c.recipients[0].id == user
						})
						.cloned()
						.unwrap_or_else(|| model::Channel {
							id: model::Id(100_000 + user.0),
							guild: None,
							name: recipient.name.clone(),
							kind: 1,
							parent_id: None,
							position: 0,
							recipients: vec![recipient],
							last_message: None,
							icon: None,
							member_list_id: None,
							message_count: None,
						});
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
					action: client_core::user_actions::Action::LoadNote(user),
					request,
				} => Event::UserAction(client_core::user_actions::Event::NoteLoaded {
					user,
					request,
					result: Ok(self.state.user_note(user).unwrap_or("").to_owned()),
				}),
				Command::UserAction { action, request } => {
					Event::UserAction(client_core::user_actions::Event::Written {
						action,
						request,
						result: Ok(()),
					})
				}
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
				} => Event::ReadState(client_core::read_state::Event::Result {
					channel,
					message,
					request,
					result: Ok(()),
				}),
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
					})
				}
				Command::Voice(_) | Command::CancelProfile | Command::CancelSearch => return,
				Command::CreatePost {
					parent,
					guild,
					title,
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
							author: message.author.name,
							excerpt: format!(
								"Synthetic pinned message: {}",
								message.content.chars().take(200).collect::<String>()
							),
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
									author: message.author.name,
									excerpt: message.content.chars().take(256).collect(),
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
				Command::ForumPosts { .. } => return,
				Command::History { before, after, .. } => {
					test_support::load_page_with_cursors(&mut self.state, before, after);
					return;
				}
				Command::Send {
					channel,
					content,
					nonce,
					reply,
				} => {
					self.synthetic_id += 1;
					let mut message = test_support::message(self.synthetic_id, channel);
					message.author = self.state.user.clone().unwrap();
					message.content = content;
					message.nonce = Some(nonce.clone());
					message.reply_to = reply;
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
				let header = egui::Frame::NONE
					.inner_margin(egui::Margin {
						left: (16.0 + ui::design::TRAFFIC_LIGHT_INSET) as i8,
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
								ui::design::window_controls,
							);
						});
					});
				ui::design::window_drag(ui, header.response.rect);
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
						ui::icons::Icon::Discord,
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
					ui.add_space(18.0);
					for detail in [self.credential_status, self.state.status]
						.into_iter()
						.filter(|detail| !detail.is_empty() && *detail != "Disconnected")
					{
						ui.add(
							egui::Label::new(egui::RichText::new(detail).size(12.0).color(p.muted))
								.wrap(),
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
								self.state.auth = AuthState::Unauthenticated;
								self.state.status = "Disconnected";
								self.credential_status = "Saved-login restore cancelled";
							}
						},
					);
				});
			});
	}
	fn sign_in_screen(&mut self, ui: &mut egui::Ui) {
		let p = ui::design::palette(ui);
		egui::CentralPanel::default()
			.frame(egui::Frame::NONE.fill(ui::design::window_palette(ui).canvas))
			.show(ui, |ui| {
				accent_glow(ui);
				let header = egui::Frame::NONE
					.inner_margin(egui::Margin {
						left: (16.0 + ui::design::TRAFFIC_LIGHT_INSET) as i8,
						right: if ui::design::WINDOW_CONTROLS_WIDTH > 0.0 {
							0
						} else {
							24
						},
						top: if ui::design::WINDOW_CONTROLS_WIDTH > 0.0 {
							0
						} else {
							18
						},
						bottom: 12,
					})
					.show(ui, |ui| {
						ui.horizontal(|ui| {
							ui.label(ui::design::semibold(ui, "Serein", 20.0).color(p.text_strong));
							ui.add_space(6.0);
							egui::Frame::NONE
								.fill(p.accent.gamma_multiply(0.16))
								.corner_radius(4)
								.inner_margin(egui::Margin::symmetric(6, 2))
								.show(ui, |ui| {
									ui.label(
										ui::design::semibold(ui, "EARLY PREVIEW", 10.0)
											.color(p.accent),
									);
								});
							ui.with_layout(
								egui::Layout::right_to_left(egui::Align::Center),
								|ui| {
									ui::design::window_controls(ui);
									ui.add_space(12.0);
									ui.menu_button("Appearance", |ui| {
										egui::widgets::global_theme_preference_buttons(ui);
										self.messaging.reading_settings(
											ui,
											self.fixture_only || self.state.demo,
										);
									});
								},
							);
						});
					});
				ui::design::window_drag(ui, header.response.rect);
				egui::ScrollArea::vertical()
					.id_salt("sign-in-scroll")
					.show(ui, |ui| {
						ui.add_space(((ui.available_height() - 560.0) * 0.4).max(8.0));
						ui.vertical_centered(|ui| {
							ui.allocate_ui_with_layout(
								egui::vec2(ui.available_width().min(460.0), 0.0),
								egui::Layout::top_down(egui::Align::Min),
								|ui| self.sign_in_card(ui),
							);
							ui.add_space(20.0);
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
	fn sign_in_card(&mut self, ui: &mut egui::Ui) {
		let ctx = ui.ctx().clone();
		let p = ui::design::palette(ui);
		let shadow = egui::epaint::Shadow {
			offset: [0, 8],
			blur: 24,
			spread: 0,
			color: egui::Color32::from_black_alpha(if ui.visuals().dark_mode { 90 } else { 30 }),
		};
		egui::Frame::NONE
			.fill(p.surface)
			.stroke(egui::Stroke::new(1.0, p.border))
			.shadow(shadow)
			.corner_radius(12)
			.inner_margin(32)
			.show(ui, |ui| {
				ui.vertical_centered(|ui| {
					let (rect, _) = ui.allocate_exact_size(egui::vec2(56.0, 56.0), egui::Sense::hover());
					ui.painter().rect_filled(rect, 14, p.accent);
					ui::icons::paint(ui.painter(), ui::icons::Icon::Discord, rect.shrink(13.0), p.accent_text);
					ui.add_space(16.0);
					ui.label(ui::design::semibold(ui, "Welcome to Serein", 24.0).color(p.text_strong));
					ui.add_space(6.0);
					ui.label(egui::RichText::new("Sign in with your Discord account to pick up where you left off.").size(15.0).color(p.muted));
				});
				ui.add_space(24.0);
				let can_sign_in = !self.fixture_only && self.authorized && !self.forgetting && self.state.auth != AuthState::Authenticating;
				let label = if self.state.auth == AuthState::Authenticating { "Waiting for Discord…" } else { "Continue with Discord" };
				let button = ui
					.add_enabled_ui(can_sign_in, |ui| {
						ui::design::primary_icon_button(ui, ui::icons::Icon::Discord, label)
					})
					.inner;
				if button.clicked() {
					if let Some(store) = &mut self.store { store.cancel_load(); }
					self.credential_status = "Sign in through Discord; saved-login lookup stopped";
					let wake = ctx.clone();
					match platform::LoginView::open(self.window.clone(), move || wake.request_repaint()) {
						Ok(login) => { self.login = Some(login); self.state.auth = AuthState::Authenticating; self.state.status = "Waiting for Discord login"; }
						Err(_) => { self.state.auth = AuthState::Failed; self.state.status = "Platform login webview unavailable; see platform-support.md"; }
					}
				}
				ui.add_space(12.0);
				ui.add_enabled_ui(!self.fixture_only, |ui| {
					ui.horizontal_wrapped(|ui| {
						ui.spacing_mut().item_spacing.x = 8.0;
						ui.checkbox(&mut self.authorized, "");
						ui.label(egui::RichText::new("I own this account and authorize this session.").size(13.0).color(p.text));
					});
				});
				ui.add_space(6.0);
				ui.label(egui::RichText::new("Discord's own login page opens inside Serein. Passwords and 2FA stay there; only the session token is kept, in your OS credential store.").size(12.0).color(p.muted));
				// Status: one banner, only when something is happening or went wrong.
				let attention = matches!(self.state.auth, AuthState::Failed | AuthState::Expired | AuthState::Challenged);
				let busy = self.state.auth == AuthState::Authenticating || self.forgetting || self.cache_pending > 0 || self.cache_clears.pending();
				let show_status = attention || busy || self.state.status != "Disconnected" || (!self.fixture_only && self.credential_status != "Checking saved login…" && !self.credential_status.is_empty());
				if show_status && !(self.fixture_only && self.state.status == "Disconnected") {
					ui.add_space(14.0);
					let (fill, color) = if attention { (p.warning.gamma_multiply(0.14), p.warning) } else { (p.raised, p.text) };
					egui::Frame::NONE.fill(fill).corner_radius(8).inner_margin(egui::Margin::symmetric(12, 10)).show(ui, |ui| {
						ui.set_width(ui.available_width());
						if self.state.status != "Disconnected" || attention {
							ui.label(egui::RichText::new(self.state.status).size(13.0).color(color));
						}
						if !self.fixture_only && !self.credential_status.is_empty() {
							ui.label(egui::RichText::new(self.credential_status).size(12.0).color(p.muted));
						}
						if self.cache_error || self.cache_pending > 0 || self.cache_clears.pending() {
							ui.label(egui::RichText::new(self.cache_status).size(12.0).color(p.muted));
						}
					});
				}
				#[cfg(feature = "demo")]
				if self.fixture_only {
				ui.add_space(22.0);
				ui.horizontal(|ui| {
					let y = ui.cursor().top() + 8.0;
					let left = ui.cursor().left();
					let width = ui.available_width();
					let galley = ui.painter().layout_no_wrap("or".into(), egui::FontId::proportional(12.0), p.muted);
					let text_w = galley.size().x + 20.0;
					ui.painter().hline(left..=left + (width - text_w) * 0.5, y, egui::Stroke::new(1.0, p.border));
					ui.painter().galley(egui::pos2(left + (width - galley.size().x) * 0.5, y - galley.size().y * 0.5), galley, p.muted);
					ui.painter().hline(left + (width + text_w) * 0.5..=left + width, y, egui::Stroke::new(1.0, p.border));
					ui.allocate_space(egui::vec2(width, 16.0));
				});
				ui.add_space(14.0);
				if ui::design::secondary_button(ui, "Explore the offline preview").clicked() {
					if let Some(store) = &mut self.store { store.cancel_load(); }
					self.connection = None;
					self.pending_save = None;
					let generation = self.state.generation + 1;
					self.state = test_support::demo_state();
					self.state.generation = generation;
					self.messaging.clear();
				}
				ui.add_space(8.0);
				ui.vertical_centered(|ui| {
					ui.label(egui::RichText::new("Sample conversations. No Discord connection.").size(12.0).color(p.muted));
				});
				}
				ui.add_space(16.0);
				ui.collapsing("About Serein", |ui| {
						ui.small("Messaging, reactions, search and read markers have offline tests. Real Discord interoperability is still unverified; attachment uploads and advanced search remain incomplete.");
						ui.small("Messages and drafts are cached locally. Login tokens use the operating system credential store.");
						ui.small("Unofficial clients may put your Discord account at risk.");
						if !self.fixture_only {
							ui.small(self.credential_status);
							if ui.button("Forget saved login").clicked() { self.logout(&ctx); }
						}
					});
				#[cfg(feature = "developer-session")]
				if !self.fixture_only {
					ui.collapsing("Developer session", |ui| {
						ui.add(egui::TextEdit::singleline(&mut *self.token_input).password(true).char_limit(2048).hint_text("Owner-supplied test credential"));
						if ui.add_enabled(self.authorized, egui::Button::new("Connect imported session (RAM only)")).clicked() {
							let input = std::mem::take(&mut *self.token_input);
							match SessionSecret::from_owner_input(input) { Ok(secret) => self.connect(secret, false, &ctx), Err(f) => self.state.status = f.label() }
						}
					});
				}
			});
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
			for _ in 0..8 {
				let Some(result) = worker.poll() else {
					break;
				};
				if let Some(error) = result.error {
					self.cache_error = true;
					self.cache_status = error;
				}
				self.messaging
					.accept_avatar(ctx, result.key.clone(), result.image);
				self.messaging
					.accept_gif_animation(result.key, result.frames);
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
		if let Some(cache) = &self.cache {
			for _ in 0..16 {
				match cache.receive.try_recv() {
					Ok(value) => cached.push(value),
					Err(_) => break,
				}
			}
		}
		for (generation, outcome, _reservation) in cached {
			self.cache_pending = self.cache_pending.saturating_sub(1);
			// Settings are global; account removal/write failures still matter after logout.
			match &outcome {
				cache::Outcome::AppPreferences(result) => {
					if !self.app_settings.state.touched {
						match result {
							Ok(value) => self.app_settings.current = value.clone(),
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
				cache::Outcome::ChannelPreferences(result) => match result {
					Ok(preferences) => self.messaging.restore_channel_preferences(preferences),
					Err(_) => {
						self.messaging.channel_preferences_status =
							"Could not restore channel shortcuts."
					}
				},
				cache::Outcome::ChannelPreferencesSaved(result) => {
					self.messaging.channel_preferences_save_pending = false;
					self.messaging.channel_preferences_status = if result.is_ok() {
						""
					} else {
						"Could not save channel shortcuts."
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
				| cache::Outcome::AppPreferences(_)
				| cache::Outcome::AppPreferencesSaved(_)
				| cache::Outcome::MinimizeToTray(_)
				| cache::Outcome::MinimizeToTraySaved(_)
				| cache::Outcome::GameActivity(_)
				| cache::Outcome::GameActivitySaved(_)
				| cache::Outcome::ReadingPreferences(_)
				| cache::Outcome::ReadingPreferencesSaved(_)
				| cache::Outcome::HistoryCleared
				| cache::Outcome::Failed { .. } => unreachable!(),
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
					let status = credentials::loaded_status(&result);
					if let Ok(Some(secret)) = result {
						self.connect(secret, false, ctx);
					}
					self.credential_status = status;
				}
				credentials::Outcome::Saved(Ok(())) => {
					self.credential_status = "Login saved in the OS credential store"
				}
				credentials::Outcome::Saved(Err(_)) => {
					self.credential_status =
						"Could not save login; this session will not restore automatically"
				}
				credentials::Outcome::Forgotten(result) => {
					self.forgetting = false;
					self.credential_status = if result.is_ok() {
						"Saved login removed"
					} else {
						"Could not remove saved login; remove org.serein.desktop / discord-session in your OS credential manager"
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
			let ready = matches!(event.event, Event::Ready { .. });
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
			if event.generation == self.state.generation
				&& (invalidate
					|| event.event.changes_access()
					|| matches!(
						&event.event,
						Event::NotificationPreferences(_)
							| Event::ChannelAction(_)
							| Event::UserAction(_)
							| Event::Disconnected | Event::ReadState(
							client_core::read_state::Event::Ack { .. }
						) | Event::ReadState(client_core::read_state::Event::Result {
							result: Ok(()),
							..
						})
					)) {
				self.notifications.dismiss();
			}
			self.state.apply(event);
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
					let preferences = &mut self.messaging.channel_preferences;
					let previous = preferences.favorites.len() + preferences.pinned.len();
					preferences.favorites.retain(|id| *id != channel);
					preferences.pinned.retain(|id| *id != channel);
					self.messaging.channel_preferences_changed |=
						previous != preferences.favorites.len() + preferences.pinned.len();
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
				if !self.fixture_only
					&& !self.state.demo
					&& let Some(command) = self.state.request_notification_settings(
						model::notification_settings::Section::Overview,
					) {
					self.command(command);
				}
				// The worker survives logout; each accepted account READY restores its own drafts.
				if !self.messaging.draft_restore_pending {
					self.messaging.draft_restore_pending =
						self.queue_cache(cache::Operation::LoadDrafts);
					self.queue_cache(cache::Operation::LoadGifFavorites);
					if !self.messaging.channel_preferences_loaded
						&& !self.queue_cache(cache::Operation::LoadChannelPreferences)
					{
						self.messaging.channel_preferences_status =
							"Could not restore channel shortcuts.";
					}
				}
				if let Some(secret) = self.pending_save.take()
					&& let Some(store) = &self.store
					&& store
						.send
						.try_send((self.state.generation, credentials::Operation::Save(secret)))
						.is_err()
				{
					self.credential_status = "Could not queue saved login; session only";
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
				let _ = store
					.send
					.try_send((self.state.generation, credentials::Operation::Forget));
			}
		}
		if let Some(login) = &self.login {
			login.pump();
			if let Some(secret) = login.token() {
				self.connect(secret, true, ctx);
			} else if login.expired() {
				self.login = None;
				self.state.auth = AuthState::Challenged;
				self.state.status =
					"Login timed out or token handoff unavailable; no session accepted";
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
	fn sync_customization(&self, ctx: &egui::Context) {
		if self.messaging.primary_color != ui::design::primary_color() {
			ui::design::set_primary_color(self.messaging.primary_color);
			ui::design::apply(ctx);
			ctx.request_repaint();
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
	fn persist_egui_memory(&self) -> bool {
		false
	}
	fn raw_input_hook(&mut self, _: &egui::Context, raw_input: &mut egui::RawInput) {
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
	}
	fn logic(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
		self.frame_metrics.begin(ctx);
		self.startup.sync(
			ctx,
			&self.runtime,
			&mut self.messaging,
			self.fixture_only || self.state.demo,
		);
		self.messaging.sync_reading_zoom(ctx);
		self.poll(ctx);
		self.state.expire_invite_challenge();
		if self.state.invite_challenge().is_some() {
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
		if let Some(tray) = &mut self.tray {
			while let Some(event) = tray.take_event() {
				match event {
					platform::tray::Event::Quit => {
						ctx.send_viewport_cmd(egui::ViewportCommand::Close)
					}
					platform::tray::Event::Show => {}
					platform::tray::Event::Unavailable => {
						self.tray_error = Some("Tray unavailable. The window will stay visible.")
					}
				}
			}
		}
		let (focused, hidden_or_closing, ptt_down) = ctx.input(|input| {
			(
				input.focused,
				input.viewport().visible() == Some(false) || input.viewport().close_requested(),
				input.key_down(egui::Key::V),
			)
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
		if let Some(kind) = self.notification_runtime.poll(
			&mut self.state,
			&mut self.messaging,
			&self.window,
			ctx,
			self.fixture_only,
		) {
			self.notifications.notify_kind(kind);
		}
		self.messaging.voice_ptt_active = focused && ptt_down && !ctx.egui_wants_keyboard_input();
	}
	fn ui(&mut self, ui: &mut egui::Ui, _: &mut eframe::Frame) {
		let ctx = ui.ctx().clone();
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
		let can_attach = self
			.state
			.selected
			.is_some_and(|channel| self.state.can_attach(channel));
		if let Some(paste) = &self.clipboard
			&& let Some(result) = paste.poll()
		{
			if paste.generation == self.state.generation
				&& Some(paste.channel) == self.state.selected
				&& self.state.user.is_some()
				&& !self.messaging.has_edit()
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
							self.state.status = error;
						}
					}
					Ok(clipboard::Content::File(..)) => {
						self.state.status = "Attaching files is unavailable here"
					}
					Err(error) => self.state.status = error,
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
				&& !self.messaging.has_edit()
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
					self.state.status = error;
				}
			} else {
				self.state.status =
					"File not attached; return to a connected conversation and drop it again";
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
		self.messaging.upload_status = self.uploads.status();
		if !self.state.demo {
			let (progress, sending) = self.uploads.transfer_progress();
			self.messaging.update_upload_progress(progress, sending);
		}
		if self.state.user.is_none() {
			self.downloads.cancel();
		}

		let download_status = match self.downloads.poll() {
			downloads::Status::Idle => String::new(),
			downloads::Status::Choosing => "Choose where to save the attachment…".into(),
			downloads::Status::Downloading { total: 0, .. } => "Loading image…".into(),
			downloads::Status::Downloading { received, total } => {
				format!("Downloading: {} / {} KiB", received / 1024, total / 1024)
			}
			downloads::Status::Saved | downloads::Status::Cancelled => String::new(),
			downloads::Status::Copied => "Copied to clipboard".into(),
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
						ui.add_space(ui::design::TRAFFIC_LIGHT_INSET);
						let (rect, _) =
							ui.allocate_exact_size(egui::vec2(32.0, 32.0), egui::Sense::hover());
						ui.painter().rect_filled(rect, 8, p.accent);
						ui::icons::paint(
							ui.painter(),
							ui::icons::Icon::Discord,
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
							ui::design::window_controls(ui);
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
			if !self
				.state
				.selected
				.is_some_and(|channel| self.state.can_attach(channel))
			{
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
					self.state.status = "Wait for the current paste to finish";
				}
			}
			if std::mem::take(&mut self.messaging.attach_requested)
				&& let Some(channel) = self.state.selected
				&& self.state.can_attach(channel)
				&& let Err(error) = self.uploads.start_choose(
					self.state.generation,
					channel,
					self.runtime.handle(),
					&ctx,
					self.window.clone(),
				) {
				self.state.status = error;
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
				if self.state.has_unsent()
					|| self.messaging.has_edit()
					|| self.messaging.has_server_settings_changes()
					|| self.state.server_settings.pending
					|| self.state.server_admin.pending
					|| self.uploads.has_unsent()
				{
					self.confirming_logout = true;
				} else {
					self.logout(&ctx);
				}
			}
		} else if let Some(stage) = self.restoring() {
			self.restoring_screen(ui, stage);
		} else {
			self.sign_in_screen(ui);
		}
		let appearance = ctx.options(|options| options.theme_preference);
		self.sync_customization(&ctx);
		self.save_app_preferences();
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
		if std::mem::take(&mut self.messaging.channel_preferences_reload) {
			self.messaging.channel_preferences_status =
				if self.queue_cache(cache::Operation::LoadChannelPreferences) {
					""
				} else {
					"Could not restore channel shortcuts."
				};
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
						"Could not save channel shortcuts."
					};
			}
		}
		if let Some(variant) = self.messaging.theme_variant_changed.take() {
			self.variant_changed = true;
			let key = (variant != ui::design::Variant::Standard).then(|| variant.key().to_owned());
			self.queue_cache_for(model::Id(0), cache::Operation::SaveThemeVariant(key));
		}
		if self.confirming_close || self.confirming_logout {
			let mut notes: Vec<&str> = Vec::new();
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
				"Leave this session?",
				"Saved text drafts survive exit; selected files must be reselected. Logging out removes local account data.",
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
						self.logout(&ctx);
					}
				}
				Some(ui::dialog::Choice::Cancelled) => {
					self.confirming_close = false;
					self.confirming_logout = false;
					self.download_close_pending = false;
				}
				None => {}
			}
		}
		self.frame_metrics.reflows = self.messaging.timeline_reflows();
		self.frame_metrics.finish();
	}
}

#[cfg(test)]
mod tests {
	use super::*;
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
		restored.kind = 2;
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
