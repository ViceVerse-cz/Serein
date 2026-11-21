//! Construct the desktop owner and restore its existing settings.
#[cfg(feature = "demo")]
use super::{
	AuthState, Command, Event, community_admin_demo, components_demo, demo_members, inbox_demo,
	onboarding_demo, polls_demo, rendering_demo, server_settings_demo, slash_demo,
};
use super::{
	Desktop, Duration, FrameMetrics, SessionEnd, State, Zeroizing, app_settings, audio, cache,
	captcha, credentials, downloads, egui, emoji_upload, extension_bridge, gpu, gpu_video,
	group_icon, plaintext_login_dir, pointer, proxy_auth, reading_settings, registered_games,
	sound_upload, startup, sticker_upload, toggle_setting, tray_window, updater, uploads, video,
	voice, voice_messages,
};
#[cfg(target_os = "windows")]
use super::{align_undecorated_surface, set_windows_taskbar_icon};

impl Desktop {
	pub(super) fn new(
		cc: &eframe::CreationContext<'_>,
		demo: bool,
		frame_sample: Option<(Duration, Duration)>,
		transparency_available: bool,
		preferences: Result<local_store::AppPreferences, local_store::StoreError>,
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
		// Configured before the worker's first read, so a plaintext login restores on launch.
		let plaintext_login = !demo
			&& preferences
				.as_ref()
				.is_ok_and(|preferences| preferences.plaintext_login);
		if !demo {
			platform::plaintext_login::configure(plaintext_login_dir(), plaintext_login);
		}
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
				} else if std::env::args().any(|arg| arg == "--demo-polls") {
					polls_demo::preview()
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
				} else if std::env::args().any(|arg| arg == "--demo-message-links") {
					test_support::message_links_demo_state()
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
		if demo && std::env::args().any(|arg| arg == "--demo-group-dm") {
			// Existing synthetic group conversation; no account or call is opened.
			let _ = state.select(model::Id(29));
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
				let fixture = demo_members(&state, None, model::Id(22), 0);
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
			let gradient_roles = std::env::args().any(|arg| arg == "--demo-role-gradients");
			for guild in state.permissions.guilds.values_mut() {
				if let Some(roles) = &mut guild.roles {
					if gradient_roles {
						for role in roles.iter_mut().filter(|role| role.id == model::Id(101)) {
							role.secondary_color = Some(0x89b4fa);
						}
					}
					roles.extend([
						model::permissions::Role {
							icon: None,
							unicode_emoji: None,
							id: model::Id(9001),
							bits: 0,
							name: "Founders".into(),
							color: 0xe78284,
							secondary_color: gradient_roles.then_some(0x89b4fa),
							tertiary_color: None,
							position: 2,
							hoist: true,
						},
						model::permissions::Role {
							icon: None,
							unicode_emoji: None,
							id: model::Id(9002),
							bits: 0,
							name: "Community".into(),
							color: if gradient_roles { 11127295 } else { 0xe5c769 },
							secondary_color: gradient_roles.then_some(16759788),
							tertiary_color: gradient_roles.then_some(16761760),
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
		let mut app_settings = app_settings::Settings::from_preferences(preferences);
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
		// Fixture video textures the view borrows by id; the desktop keeps them alive.
		#[cfg_attr(not(feature = "demo"), allow(unused_mut))]
		let mut demo_textures: Vec<egui::TextureHandle> = Vec::new();
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
		app_settings.apply(&mut messaging);
		ui::design::set_reduced_motion(&cc.egui_ctx, messaging.reduce_interface_motion);
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
			gpu_video::install(render_state);
			let wake = cc.egui_ctx.clone();
			platform::video::denoise::initialize_gpu(
				render_state.device.clone(),
				render_state.queue.clone(),
				std::sync::Arc::new(move || wake.request_repaint()),
			);
		}
		let video_adapter = cc
			.wgpu_render_state
			.as_ref()
			.map(|render_state| gpu::video_adapter(&render_state.adapter))
			.unwrap_or_default();
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
			// Watching enlarges the share, as a click on Watch Stream does.
			if state.watch_stream(peer).is_some() {
				messaging.voice_focus = Some(ui::StageFocus::Stream(peer));
			}
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
			let texture =
				cc.egui_ctx
					.load_texture("synthetic-stream", image, egui::TextureOptions::LINEAR);
			messaging.voice_stream_view = Some(egui::load::SizedTexture::from_handle(&texture));
			demo_textures.push(texture);
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
			let texture = cc.egui_ctx.load_texture(
				"synthetic-remote-camera",
				image,
				egui::TextureOptions::LINEAR,
			);
			messaging
				.voice_remote_video
				.push((robin, egui::load::SizedTexture::from_handle(&texture)));
			demo_textures.push(texture);
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
				state.members = Some(demo_members(&state, list.guild, list.channel, list.request));
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
			if std::env::args().any(|arg| arg == "--demo-external-upload")
				&& let Some(channel) = state.selected
			{
				messaging.external_upload.open(
					state.generation,
					channel,
					0,
					None,
					"quarterly-report.pdf".into(),
					1_482_311,
				);
			}
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
				state.failure_detail =
					Some("guilds[3].channels[12].permission_overwrites[0].allow: invalid type: null, expected a string".into());
			}
		}
		#[cfg(feature = "demo")]
		if demo && std::env::args().any(|arg| arg.starts_with("--demo-onboarding")) {
			onboarding_demo::open(&mut state);
		}
		#[cfg(feature = "demo")]
		if demo
			&& std::env::args().any(|arg| arg == "--demo-server-invite")
			&& let Some(guild) = state
				.selected
				.and_then(|id| state.channels.iter().find(|c| c.id == id))
				.and_then(|c| c.guild)
		{
			messaging.preview_server_invite(&mut state, guild);
		}
		#[cfg(feature = "demo")]
		if demo && std::env::args().any(|arg| arg == "--demo-server-settings") {
			server_settings_demo::open(&mut state, &mut messaging);
		}
		// With `--demo-server-settings --demo-server-page=soundboard`, `--demo-sound-upload`
		// adds the upload review with a synthetic 77.8-second waveform. No file or audio
		// device is used.
		#[cfg(feature = "demo")]
		if demo && std::env::args().any(|arg| arg == "--demo-sound-upload") {
			let peaks = (0..7780u32)
				.map(|n| {
					let t = n as f32 / 7780.0;
					let envelope = 0.35 + 0.65 * (t * 9.0).sin().abs();
					let detail = 0.55 + 0.45 * ((n as f32 * 0.37).sin() * (n as f32 * 0.11).cos());
					(envelope * detail * 235.0).clamp(12.0, 255.0) as u8
				})
				.collect();
			messaging.preview_server_sound_upload(("Synthetic song".into(), 77_800, peaks, None));
		}
		#[cfg(feature = "demo")]
		if demo
			&& std::env::args().any(|arg| arg == "--demo-server-notifications")
			&& let Some(guild) = state.guilds.first().map(|g| g.id)
		{
			// The new editor fixture includes a known server default: mentions only.
			state.guilds[0].default_message_notifications = Some(1);
			messaging.preview_server_notifications(&mut state, guild);
		}
		#[cfg(feature = "demo")]
		if demo
			&& std::env::args().any(|arg| arg == "--demo-member-guide")
			&& let Some(guild) = state.guilds.first().map(|g| g.id)
			&& let Some(Command::MemberGuide {
				guild,
				user,
				request,
				action,
			}) = state.open_server_guide(guild)
		{
			let event = community_admin_demo::execute_member(&state, guild, user, request, action);
			state.apply(client_core::Envelope {
				generation: state.generation,
				event,
			});
		}
		let mut hotkeys = platform::hotkeys::Hotkeys::new({
			let ctx = cc.egui_ctx.clone();
			move || ctx.request_repaint()
		});
		if !demo {
			hotkeys.sync(&messaging.keybinds, &runtime);
		}
		let system_theme = platform::system_theme::SystemTheme::watch(&runtime, {
			let ctx = cc.egui_ctx.clone();
			move || ctx.request_repaint()
		});
		let window = cc
			.winit_window()
			.ok_or("Native window unavailable")?
			.clone();
		messaging.hide_window_decorations = cfg!(target_os = "linux") && !window.is_decorated();
		app_settings.current.hide_window_decorations = messaging.hide_window_decorations;
		#[cfg(target_os = "windows")]
		{
			use winit::platform::windows::{CornerPreference, WindowExtWindows as _};
			// egui sets ICON_SMALL; winit otherwise leaves ICON_BIG empty for the taskbar.
			set_windows_taskbar_icon(&window);
			window.set_corner_preference(CornerPreference::Round);
			align_undecorated_surface(&window);
		}
		// The GPU surface and X11 visual are selected at startup. Opaque launches
		// keep the same native/compositor path as builds without window effects.
		// KWin hiding runs its D-Bus worker on the application runtime.
		let tray_window = {
			let _runtime = runtime.enter();
			tray_window::State::default()
		};
		#[cfg(target_os = "macos")]
		let app_menu = {
			let wake = cc.egui_ctx.clone();
			platform::app_menu::AppMenu::new(
				window.clone(),
				move || wake.request_repaint(),
				ui::i18n::translate,
			)
			.ok()
			.map(|menu| (menu, ui::i18n::Language::System))
		};
		Ok(Self {
			#[cfg(feature = "demo")]
			inbox_demo: inbox_demo::Store::default(),
			proxy_auth: proxy_auth::Authentication::default(),
			api_proxy: tokio::sync::watch::channel(None).0,
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
			video: video::Video::for_adapter(video_adapter),
			_demo_textures: demo_textures,
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
			voice_messages: voice_messages::VoiceMessages::default(),
			interaction_files: Default::default(),
			group_icon: group_icon::GroupIcon::default(),
			create_server_icon: group_icon::GroupIcon::default(),
			profile_avatar: group_icon::GroupIcon::default(),
			server_profile_image: group_icon::GroupIcon::default(),
			server_icon: group_icon::GroupIcon::default(),
			community_artwork: group_icon::GroupIcon::default(),
			community_artwork_scope: None,
			webhook_avatar: group_icon::GroupIcon::default(),
			webhook_avatar_scope: None,
			role_icon: group_icon::GroupIcon::default(),
			role_icon_scope: None,
			emoji_upload: emoji_upload::EmojiUpload::default(),
			sticker_upload: sticker_upload::StickerUpload::default(),
			sound_upload: sound_upload::SoundUpload::default(),
			clipboard: None,
			download_close_pending: false,
			window_blur: transparency_available
				.then(|| platform::window_effects::Blur::new(window.clone())),
			window,
			cached_history_request: Default::default(),
			force_sync: Default::default(),
			member_cache_load: Default::default(),
			member_cache_search: Default::default(),
			expression_cache: Default::default(),
			sticker_pack_cache_load: Default::default(),
			sticker_pack_cache_save: Default::default(),
			gif_cache: Default::default(),
			member_cache_saved: None,
			member_cache_cleared: Default::default(),
			server_cache_load: Default::default(),
			server_cache_failed: false,
			maximize_after_show: false,
			monitor_geometry: None,
			monitor_period: None,
			frame_metrics: FrameMetrics::new(frame_sample),
			message_speaker: None,
			message_speech_target: None,
			timezone_focused: false,
			timezone_checked: std::time::Instant::now(),
			#[cfg(feature = "demo")]
			rendering_demo: (demo && std::env::args().any(|arg| arg == "--demo-rendering"))
				.then(rendering_demo::RenderingDemo::default),
			avatars: None,
			avatar_cleanup: None,
			avatar_start_failed: false,
			avatar_clear_account: None,
			voice: voice::Voice::for_adapter(video_adapter),
			runtime,
			store,
			cache,
			cache_pending,
			cache_clears: Default::default(),
			cache_forgets: Default::default(),
			cache_error: false,
			cache_status: "Loading local appearance…",
			appearance: egui::ThemePreference::System,
			appearance_changed: false,
			transparency_available,
			window_transparent: transparency_available,
			reading,
			app_settings,
			font_picker: None,
			font_families: None,
			updater: updater::Updater::new(demo),
			game_activity,
			game_mentions: Default::default(),
			registered_games: if demo {
				registered_games::Registered::default()
			} else {
				registered_games::Registered::load()
			},
			tray_setting,
			startup,
			tray: None,
			tray_window,
			#[cfg(target_os = "macos")]
			app_menu,
			instance: Default::default(),
			hotkeys,
			system_theme,
			tray_error: None,
			#[cfg(feature = "demo")]
			demo_typing,
			variant_changed: false,
			pending_save: None,
			pending_account_save: None,
			current_login: None,
			plaintext_login,
			plaintext_save_pending: false,
			account_presences: std::collections::BTreeMap::new(),
			presence_load_pending,
			deferred_connect: None,
			presence_authoritative: false,
			presence_saved: None,
			confirming_forget: sign_in_forget,
			sign_in_copied: None,
			credential_status: if demo {
				"Fixture mode skips saved login; extensions use public GitHub downloads"
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
			#[cfg(feature = "demo")]
			demo_groups: Default::default(),
			token_input: Zeroizing::new(String::new()),
		})
	}
}
