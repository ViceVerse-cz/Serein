#![allow(clippy::items_after_test_module)]
//! Native egui views; emits commands without owning transports or session credentials.
mod account_badge;
mod account_menu;
mod account_settings;
mod activity_alerts;
mod announcements;
#[cfg(test)]
mod appearance_transparency_tests;
mod archives;
mod audio;
mod badges;
mod composer;
pub mod force_sync;
mod forwarding;
mod owned_cosmetics;
mod panels;
mod privacy_settings;
mod role_names;
mod server_profile;
#[cfg(test)]
mod test_ui;
mod view;
pub use audio::{AudioCommand, AudioState, AudioUi};
mod video;
pub use video::{
	MediaView, VideoCommand, VideoState, VideoUi, embedded_video_candidates, embedded_video_url,
};
mod attachments;
pub mod external_upload;
pub mod voice_messages;
pub use attachments::DownloadUi;
mod avatars;
#[cfg(feature = "demo")]
pub use avatars::media::debug_heic_layout_check;
pub use avatars::media::{Lane, MAX_FRAMES, Motion, Rendition, Size, fit_edge, is_motion_video};
pub use avatars::{EMBED_EDGE, FRAME_EDGE, GifFrames};
mod browse_channels;
mod categories;
#[cfg(feature = "demo")]
pub use categories::debug_thread_navigation_check;
#[cfg(feature = "demo")]
pub use timeline::debug_unread_navigation_check;
mod channel_marks;
mod channel_menu;
mod channel_permissions;
#[cfg(test)]
mod channel_welcome_tests;
mod components;
mod composer_text;
#[cfg(any(test, all(debug_assertions, feature = "demo")))]
mod recovery_demo;
#[cfg(all(debug_assertions, feature = "demo"))]
pub use recovery_demo::check as debug_resume_send_check;
pub mod design;
mod embeds;
mod extension_account_actions;
mod extension_actions;
mod extension_admin_actions;
mod extension_app;
mod extension_server_actions;
mod extensions_ui;
pub mod proxy_auth;
mod rich_presence;
mod theme_editor;
mod thread_create;
mod thread_members;
pub use extensions_ui::{ExtensionContext, ExtensionEntry, ExtensionRequest, ExtensionUi};
pub mod emoji;
mod emoji_details;
mod emoji_picker;
mod emoticons;
pub mod fonts;
pub mod i18n;
#[cfg(all(debug_assertions, feature = "demo"))]
pub fn debug_channel_creation(state: client_core::State) {
	channel_menu::debug_creation(state);
}
mod channel_pill;
mod community;
mod formatting;
mod forum;
mod forum_settings;
mod friends;
mod group_members;
mod group_menu;
mod guild_folders;
mod highlight;
pub mod icons;
mod inbox;
mod invites;
pub mod local_time;
mod markdown;
mod member_search;
mod mentions;
mod messaging_permissions;
mod notification_settings;
mod notifications;
mod pending;
mod post_menu;
mod preload;
mod profiles;
mod slash_builtin;
mod slash_commands;
mod stickers;
/// Synthetic global profile used exclusively by the desktop's offline command adapter.
#[cfg(any(test, feature = "demo"))]
pub fn synthetic_own_profile(user: &model::User) -> model::UserProfile {
	profiles::synthetic(user, None)
}
mod community_admin;
mod contact_editor;
pub mod dialog;
mod join_server;
mod keybinds;
mod member_guide;
mod message_menu;
pub use message_menu::MessageSpeech;
mod command_permissions;
mod mod_view;
mod moderation_bulk;
mod name_styles;
mod onboarding;
mod polls;
mod profile_edit;
mod reactions;
mod reading;
mod registered_games;
mod restricted;
mod role_preview;
pub mod screen;
pub mod scroll;
mod search;
#[cfg(test)]
mod search_navigation_tests;
pub mod select;
mod server_access;
mod server_admin;
mod server_audit_log;
mod server_automod;
mod server_integrations;
mod server_invite;
mod server_invites;
mod server_menu;
mod server_moderation;
#[cfg(test)]
mod server_notification_tests;
mod server_notifications;
mod server_roles;
mod server_settings;
mod server_sounds;
mod server_stickers;
mod settings;
mod shortcuts;
mod switcher;
mod text_menu;
mod thumbhash;
#[cfg(feature = "demo")]
pub use polls::debug_poll_check;
mod mfa;
mod timeline;
#[cfg(test)]
mod title_bar_tests;
pub mod toasts;
mod typing;
pub mod updates;
mod user_direct;
mod user_menu;
mod verification;
mod voice;
mod voice_connection;
use client_core::{Command, MAX_DRAFT_BYTES, NavStep, State};
use egui::{RichText, TextEdit};
use model::{Freshness, Id};
pub use verification::VerificationUi;
pub use voice::StageFocus;
pub use voice_connection::VoiceDiagnostics;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct VoiceGain {
	pub input_percent: u16,
	pub output_percent: u16,
}
impl Default for VoiceGain {
	fn default() -> Self {
		Self {
			input_percent: 100,
			output_percent: 100,
		}
	}
}

#[derive(Clone)]
pub struct PasteTarget {
	pub generation: u64,
	pub channel: Id,
	pub widget: egui::Id,
	pub text: String,
	pub cursor: Option<egui::text::CCursorRange>,
	pub text_attachment: bool,
}

impl PasteTarget {
	pub fn replacement(&self, pasted: &str) -> String {
		composer::paste_replacement(&self.text, self.cursor, pasted)
	}
}

pub struct AttachmentPaste {
	pub target: PasteTarget,
	pub text: Option<String>,
	pub image: Option<std::sync::Arc<egui::ColorImage>>,
}

fn oversized_text(limit: u64) -> String {
	crate::i18n::translate_args(
		"public-upload-oversized",
		&[("limit", &attachments::format_size(limit))],
	)
}

fn thread_member_rows<'a>(
	state: &'a State,
	list: &'a model::MemberList,
) -> Vec<(std::borrow::Cow<'a, model::MemberSlot>, Option<u64>)> {
	use std::borrow::Cow;
	let mut members: Vec<_> = list
		.slots
		.iter()
		.flatten()
		.filter_map(|slot| {
			let model::MemberSlot::Person(member) = slot else {
				return None;
			};
			let offline = !profiles::member_presence(state, member, list.guild)
				.0
				.is_some_and(|status| matches!(status, "online" | "idle" | "dnd"));
			let role = list
				.guild
				.and_then(|guild| state.member_roles(guild, member).0)
				.filter(|_| !offline);
			Some((slot, offline, role))
		})
		.collect();
	members.sort_by(|a, b| {
		a.1.cmp(&b.1).then_with(|| match (a.2, b.2) {
			(Some(a), Some(b)) => b.cmp_hierarchy(a),
			(Some(_), None) => std::cmp::Ordering::Less,
			(None, Some(_)) => std::cmp::Ordering::Greater,
			_ => std::cmp::Ordering::Equal,
		})
	});
	let mut rows = Vec::with_capacity(members.len() * 2);
	for group in members.chunk_by(|a, b| a.1 == b.1 && a.2.map(|r| r.id) == b.2.map(|r| r.id)) {
		let id = if group[0].1 {
			"offline".into()
		} else {
			group[0]
				.2
				.map_or_else(|| "online".into(), |role| role.id.to_string())
		};
		rows.push((
			Cow::Owned(model::MemberSlot::Group(id)),
			Some(group.len() as u64),
		));
		rows.extend(group.iter().map(|row| (Cow::Borrowed(row.0), None)));
	}
	rows
}

struct SubmittedEdit {
	channel: Id,
	message: Id,
	draft: String,
	content: String,
}

#[derive(Default)]
pub struct MessagingUi {
	/// Explicit requests; the desktop owns synchronization and persistence.
	pub force_sync_action: Option<force_sync::Action>,
	pub force_sync_progress: Option<force_sync::Progress>,
	pub force_sync_cache: Option<force_sync::CacheSummary>,
	pub force_sync_open: bool,
	message_speech: Option<message_menu::MessageSpeech>,
	message_speech_error: Option<&'static str>,
	poll_creator: polls::Creator,
	pub language: i18n::Language,
	forwarding: forwarding::ForwardDialog,
	pub convert_emoticons: bool,
	/// Device preference: request a conversation's page while the pointer rests on it.
	pub preload_on_hover: bool,
	preload: preload::Preload,
	pub image_share_requested: Option<(u64, Id, Vec<model::ImageShare>, String)>,
	pub interaction_file_request: Option<String>,
	interaction_components: components::Components,
	pub verification: VerificationUi,
	mfa: mfa::MfaUi,
	onboarding: onboarding::OnboardingUi,
	pub extensions: ExtensionUi,
	friends: friends::Friends,
	inbox_open: bool,
	inbox_controls: inbox::Controls,
	account_menu: account_menu::AccountMenu,
	pub own_presence: model::OwnPresence,
	/// When the custom status clears itself, in milliseconds since the Unix epoch. The
	/// deadline is local: Discord carries it in account settings, which this client does
	/// not write, so the host enforces it and republishes the cleared presence.
	pub own_presence_expires: Option<u64>,
	pub own_presence_changed: bool,
	pub own_presence_status: &'static str,
	group_menu: group_menu::GroupMenu,
	group_create: group_members::NewGroup,
	server_menu: server_menu::ServerMenu,
	browse_channels: browse_channels::Browser,
	navigation_browser: Option<Id>,
	community_ui: community::CommunityUi,
	channel_menu: channel_menu::ChannelMenu,
	pub channel_preferences: model::ChannelPreferences,
	pub channel_preferences_changed: bool,
	pub channel_preferences_loaded: bool,
	pub channel_preferences_load_pending: bool,
	pub channel_preferences_reload: bool,
	pub channel_preferences_save_pending: bool,
	pub channel_preferences_status: &'static str,
	/// Saved per-server channels still need to reach this session's navigation memory.
	last_channels_seed: bool,
	last_channel_recorded: Option<Id>,
	join_server: join_server::JoinDialog,
	folder_ui: guild_folders::FolderUi,
	rail_cache: notifications::RailCache,
	composer_layout: composer_text::Layout,
	channel_cache: categories::Cache,
	channel_move: Option<(Id, client_core::channel_actions::Action)>,
	search: search::SearchUi,
	settings: settings::Settings,
	server_settings: server_settings::Editor,
	mod_view: mod_view::ModViewUi,
	server_icon_sequence: u64,
	server_emoji_sequence: u64,
	server_sticker_sequence: u64,
	server_role_icon_sequence: u64,
	switcher: switcher::Switcher,
	focus_switched_composer: bool,
	#[cfg(feature = "demo")]
	preview_slash_commands: bool,
	switcher_frame: bool,
	archives: archives::ArchivesUi,
	archive_parent: Option<Id>,
	thread_create: thread_create::ThreadCreateUi,
	announcement_follow: announcements::FollowDialog,
	forum: forum::ForumUi,
	scroll: scroll::Session,
	side_press: scroll::SidePress,
	timeline: timeline::TimelineView,
	edit_modified: Option<(Id, Id, bool)>,
	edit_undo_cleared: bool,
	edit_widget_id: Option<egui::Id>,
	edit_closed_channel: Option<Id>,
	avatars: avatars::Avatars,
	profile: profiles::ProfileSession,
	profile_image: Option<(u64, model::Attachment)>,
	user_action: Option<user_menu::Action>,
	pending_mention: Option<Id>,
	contact_editor: contact_editor::ContactEditor,
	profile_link: Option<String>,
	profile_formatted: markdown::FormatCache,
	pub reading_preferences: model::ReadingPreferences,
	pub show_hidden_channels: bool,
	/// Folds the home list's message requests away; session-local like hover state.
	message_requests_collapsed: bool,
	/// Custom emoji that need Nitro stay out of `:` suggestions and are locked in pickers.
	pub hide_nitro_emojis: bool,
	/// Profile cosmetics shown on this device; hidden ones are not downloaded.
	pub cosmetics: model::CosmeticPreferences,
	/// Disk budgets and retention for downloaded images.
	pub image_cache: model::CachePreferences,
	/// Clock style for message times.
	pub time_format: model::TimeFormat,
	/// Device-local display timezone, independent of language and clock style.
	pub time_zone: model::TimeZonePreference,
	/// Saved logins may fall back to plain files when no OS keyring works.
	pub plaintext_login: bool,
	/// Latest disk use of the media and shared-artwork caches, once measured.
	pub image_cache_usage: Option<[model::CacheUsage; 2]>,
	/// An image cache partition the owner asked to empty; the desktop consumes it.
	pub clear_image_cache: Option<model::CachePartition>,
	pub hide_title_bar: bool,
	pub hide_window_decorations: bool,
	pub custom_font: fonts::Settings,
	/// Which GPU renders the window; the running adapter only changes on restart.
	pub gpu_preference: model::GpuPreference,
	/// Adapter currently in use, shown next to the preference for bug reports.
	pub gpu_adapter: String,
	pub reading_status: &'static str,
	pub reading_save_requested: bool,
	pub minimize_to_tray: bool,
	pub tray_available: bool,
	pub tray_status: &'static str,
	pub startup_enabled: bool,
	pub startup_minimized: bool,
	pub startup_available: bool,
	pub startup_busy: bool,
	pub startup_status: &'static str,
	pub startup_disable_requested: bool,
	pub discord_activity_sharing: Option<bool>,
	pub discord_activity_sharing_busy: bool,
	pub discord_activity_sharing_retry: bool,
	/// false checks the account setting; true explicitly enables it.
	pub discord_activity_sharing_request: Option<bool>,
	pub share_game_activity: bool,
	pub own_game: Option<String>,
	pub game_activity_status: &'static str,
	/// Edited on the Registered Games page; the desktop app persists and applies the list.
	pub registered_games: Vec<model::registered_games::RegisteredGame>,
	/// The game the local process scan reports while activity sharing is on.
	pub running_game: Option<model::registered_games::RunningGame>,
	/// Set by "Add it!"; the desktop app answers in `running_processes`.
	pub running_processes_request: bool,
	pub running_processes: Option<Vec<String>>,
	reading_sidebar_applied: Option<u16>,
	reading_sidebar_constrained: bool,
	reading_zoom_pending: bool,
	/// Slider value while the pointer is still down; zoom is applied on release so the
	/// slider does not rescale under the cursor mid-drag.
	reading_zoom_draft: Option<u16>,
	friend_removal: Option<(u64, model::User)>,
	user_direct: Option<user_direct::Pending>,
	members_narrow_open: bool,
	/// Only the open member request's revealed prefix; member data stays in the bounded core cache.
	member_extent: Option<((u64, Id, u64), usize)>,
	guild: Option<Id>,
	navigation_channel: Option<Id>,
	pub logout_requested: bool,
	/// Accounts remembered on this device, most recently used first.
	pub accounts: Vec<model::SavedAccount>,
	/// Switch to this saved account; the host ends the session and reads its saved token.
	pub switch_account_requested: Option<Id>,
	/// End the session and return to the sign-in screen to remember one more account.
	pub add_account_requested: bool,
	/// Drop one saved account: its token, cached history and drafts.
	pub forget_account_requested: Option<Id>,
	pub reconnect_requested: bool,
	/// Fixed-label, session-only connection history prepared by the desktop transport.
	pub connection_diagnostics: String,
	pub connection_reason: Option<&'static str>,
	pub draft_changes: Vec<Id>,
	pub draft_restore_pending: bool,
	/// The reply target's author, kept while that message leaves the loaded window.
	reply_author: Option<(Id, String)>,
	pub attachment: Option<(String, u64)>,
	pub attachment_files: Vec<(String, u64)>,
	pub remove_attachment_index: Option<usize>,
	/// Downscaled pixels per selected file (`None` for non-images), produced off the render thread.
	pub attachment_previews: Vec<Option<std::sync::Arc<egui::ColorImage>>>,
	/// GPU copies of `attachment_previews`, keyed by the pixel buffer they were uploaded from.
	attachment_textures: Vec<Option<(usize, egui::TextureHandle)>>,
	pub attach_requested: bool,
	pub host_attachment_requested: Option<usize>,
	pub attachment_paste_requested: Option<AttachmentPaste>,
	pub pasted_text: Option<(PasteTarget, String)>,
	paste_key_handled: bool,
	/// A text field whose context menu asked to paste.
	menu_paste: Option<(u64, Id, egui::Id)>,
	pub remove_attachment_requested: bool,
	pub cancel_upload_requested: bool,
	pub upload_busy: bool,
	/// Composer attachments cannot be selected or removed right now. A native picker with
	/// an unknown count blocks edits; known loading files and a previous upload do not.
	pub attach_busy: bool,
	/// Selected files still being inspected; shown as loading tiles in the upload tray.
	pub attachment_loading: usize,
	pub external_upload: external_upload::ExternalUpload,
	pub voice_messages: voice_messages::VoiceMessages,
	/// Transient problem and progress notices. Nothing here outlives its deadline.
	pub toasts: toasts::Toasts,
	pending_upload: Option<pending::Upload>,
	pub clear_cache_requested: bool,
	pub voice_available: bool,
	pub voice_switch_ready: bool,
	voice_switch: Option<voice::CallSwitch>,
	pub screen: screen::ScreenUi,
	pub voice_camera_available: bool,
	pub voice_camera_status: &'static str,
	/// Availability of the hardware-only camera filter on the renderer's GPU.
	pub camera_denoising_availability: model::voice_settings::CameraDenoisingAvailability,
	/// Runtime denoising state for the outgoing camera and settings preview, respectively.
	pub call_camera_denoising_status: Option<model::voice_settings::CameraDenoisingStatus>,
	pub preview_camera_denoising_status: Option<model::voice_settings::CameraDenoisingStatus>,
	pub voice_cameras: Vec<(String, String)>,
	pub voice_camera_device: Option<String>,
	pub voice_refresh_cameras: bool,
	pub voice_camera_device_status: &'static str,
	pub voice_camera_devices_loading: bool,
	pub voice_camera_preview: Option<egui::TextureHandle>,
	pub camera_test_requested: bool,
	pub camera_test_available: bool,
	pub camera_test_status: &'static str,
	pub camera_test_texture: Option<egui::TextureHandle>,
	/// Decoded remote cameras by user; the desktop owns, bounds and replaces the textures.
	pub voice_remote_video: Vec<(Id, egui::load::SizedTexture)>,
	/// Latest picture of the screen share this device chose to watch; desktop-owned texture.
	pub voice_stream_view: Option<egui::load::SizedTexture>,
	pub voice_stream_hdr: Option<MediaView>,
	pub voice_remote_hdr: Vec<(Id, MediaView)>,
	pub voice_stream_status: &'static str,
	/// Session-only stream playback level; unset is 100%. Mute preserves the level.
	voice_stream_volume: Option<u16>,
	voice_stream_muted: bool,
	/// Session-only soundboard playback level; unset is 100%.
	voice_soundboard_volume: Option<u16>,
	/// Enlarged stage tile; cleared when it stops showing video or on Escape.
	pub voice_focus: Option<voice::StageFocus>,
	/// Screen share currently owning the client surface, its prior window mode and focus target.
	voice_fullscreen: Option<(voice::StageFocus, egui::Context, bool, egui::Id)>,
	/// Native window transition for the desktop to apply after this UI frame.
	voice_fullscreen_request: Option<bool>,
	/// Whether the other participants stay visible as a strip under the enlarged tile.
	pub voice_focus_participants: bool,
	/// How far the call stage's hover chrome (and tile name badges) has faded out: 0 shown.
	voice_chrome_hidden: f32,
	/// Session-only visibility of the selected guild voice channel's chat.
	pub voice_chat_open: bool,
	/// Tile click to start (`Some(user)`) or stop (`None`) watching, applied by the stage.
	watch_request: Option<Option<Id>>,
	ring_request: Option<(Id, u64, Id, bool)>,
	voice_moderation: Option<(Id, Id, Id, model::server_admin::VoiceChange)>,
	stream_preview_open: Option<((Id, Id, Id), u64)>,
	stream_preview_request: Option<(Id, Id, Id)>,
	stream_preview_watch: Option<(Id, Id)>,
	pub voice_inputs: Vec<(String, String)>,
	pub voice_outputs: Vec<(String, String)>,
	pub voice_input: Option<String>,
	pub voice_output: Option<String>,
	pub media_output: Option<String>,
	pub voice_gain: VoiceGain,
	voice_user_volumes: Option<Box<[(u64, u16); 64]>>,
	/// Speakers silenced on this device only; never sent to Discord.
	voice_user_muted: Vec<u64>,
	pub voice_refresh_devices: bool,
	pub voice_device_status: &'static str,
	pub voice_microphone_unavailable: bool,
	pub voice_push_to_talk: bool,
	/// Device-local voice intent, retained between calls and restarts.
	pub voice_muted: bool,
	pub voice_deafened: bool,
	pub voice_processing: model::voice_settings::VoiceProcessing,
	pub video_settings: model::voice_settings::VideoSettings,
	/// Rendered visibility of the advanced panel, used to retire hidden probes.
	pub video_encoding_advanced_visible: bool,
	/// Session-local driver reports; the host queries outside the render thread.
	pub video_capabilities: Option<model::voice_settings::DriverCapabilities>,
	pub video_capabilities_loading: bool,
	pub video_capabilities_refresh: bool,
	/// Explicit synthetic encode results, kept separate from driver codec support.
	pub video_encoder_tests: Option<model::voice_settings::VideoCapabilities>,
	pub video_encoder_tests_loading: bool,
	pub video_encoder_tests_request: Option<model::voice_settings::VideoCodec>,
	pub video_encoder_tests_preset: Option<model::voice_settings::EncoderPreset>,
	pub video_encoder_test_presets: [Option<model::voice_settings::EncoderPreset>; 3],
	/// Codec of the active or last test, independent of changes to the selected codec.
	pub video_encoder_tests_codec: Option<model::voice_settings::VideoCodec>,
	pub voice_preview_requested: bool,
	pub voice_preview_level: Option<f32>,
	pub voice_preview_status: &'static str,
	pub voice_lavasr_status: Option<&'static str>,
	/// Device-local application shortcuts; the desktop host mirrors the global binding.
	pub keybinds: model::Keybinds,
	keybind_capture: Option<model::KeybindAction>,
	pub global_keybind_status: &'static str,
	/// Server folders the owner left open; restored from device preferences at startup.
	pub expanded_folders: Vec<u64>,
	pub voice_ptt_active: bool,
	pub voice_ptm_active: bool,
	pub voice_privacy_code: Option<String>,
	pub voice_diagnostics: VoiceDiagnostics,
	voice_connection_view: voice_connection::ConnectionView,
	/// Latest media activity, capped at 64 IDs (512 bytes) by the voice host.
	pub voice_speaking: Vec<Id>,
	pub notifications_enabled: bool,
	pub notification_options: model::notification_preferences::Device,
	pub notification_preview: Option<model::notification_preferences::Sound>,
	pub notification_sound_status: &'static str,
	pub notification_test_available: bool,
	pub notification_test_requested: bool,
	pub notification_status: &'static str,
	pub storage_status: &'static str,
	/// Release channel and version shown in the title bar.
	pub build: design::Build,
	pub updates: updates::Updates,
	pub updates_save_failed: bool,
	/// Header pin button rect while the pins popout is open.
	pins_anchor: Option<egui::Rect>,
	editing: Option<(Id, Id, String)>,
	composer_edit: Option<(Id, Id)>,
	edit_sent: Option<SubmittedEdit>,
	deleting: Option<(Id, Id)>,
	publishing: Option<(u64, Id, Id)>,
	ime_active: bool,
	/// The composer claimed Tab last frame, so egui left it in the input for a suggestion menu.
	composer_claims_tab: bool,
	mention_menu: mentions::Menu,
	slash_commands: slash_commands::Menu,
	slash_direct: Option<slash_builtin::PendingMessage>,
	mention_lookup: member_search::Search,
	emoji_picker: emoji_picker::Picker,
	reaction_picker: emoji_picker::Picker,
	/// Set when the user picks a theme preset; the host persists it.
	pub theme_variant_changed: Option<design::Variant>,
	pub primary_color: Option<[u8; 3]>,
	pub reduce_interface_motion: bool,
	pub transparency_blur: bool,
	pub transparency: u8,
	pub blur: u8,
}

/// Context strip (reply/edit) drawn as the rounded top of the composer block.
fn composer_cap(
	ui: &mut egui::Ui,
	colors: &design::Palette,
	add_contents: impl FnOnce(&mut egui::Ui),
) -> egui::Rect {
	design::glass_frame(
		ui,
		design::mix(colors.raised, colors.accent, 0.16),
		egui::Margin {
			left: 16,
			right: 8,
			top: 5,
			bottom: 5,
		},
	)
	.corner_radius(egui::CornerRadius {
		nw: 8,
		ne: 8,
		sw: 0,
		se: 0,
	})
	.show(ui, |ui| {
		ui.set_min_width((ui.available_width()).max(0.0));
		ui.horizontal(|ui| {
			ui.with_layout(
				egui::Layout::right_to_left(egui::Align::Center),
				add_contents,
			);
		});
	})
	.response
	.rect
}

/// Retain the full search field on wide headers and a focusable icon on narrow ones.
fn header_search_button(ui: &mut egui::Ui, enabled: bool, label: &str) -> egui::Response {
	ui.add_enabled_ui(enabled, |ui| {
		if ui.available_width() < 440.0 {
			return icons::button(ui, icons::Icon::Search, 32.0, label);
		}
		let colors = design::palette(ui);
		let (pill, response) =
			ui.allocate_exact_size(egui::vec2(144.0, 28.0), egui::Sense::click());
		response
			.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), label));
		let hot = ui.is_enabled() && (response.hovered() || response.has_focus());
		ui.painter()
			.rect_filled(pill, 6, if hot { colors.hover } else { colors.raised });
		if response.has_focus() {
			ui.painter().rect_stroke(
				pill,
				6,
				egui::Stroke::new(1.0, colors.accent),
				egui::StrokeKind::Inside,
			);
		}
		let color = if ui.is_enabled() {
			colors.muted
		} else {
			colors.muted.gamma_multiply(0.5)
		};
		let mut job = egui::text::LayoutJob::simple_singleline(
			label.to_owned(),
			egui::FontId::proportional(13.0),
			color,
		);
		job.wrap = egui::text::TextWrapping::truncate_at_width(pill.width() - 40.0);
		let galley = ui.painter().layout_job(job);
		ui.painter().galley(
			pill.left_center() + egui::vec2(10.0, -galley.size().y * 0.5),
			galley,
			color,
		);
		icons::paint(
			ui.painter(),
			icons::Icon::Search,
			egui::Rect::from_center_size(
				pill.right_center() - egui::vec2(14.0, 0.0),
				egui::Vec2::splat(16.0),
			),
			color,
		);
		response
	})
	.inner
}

fn mention_switch(ui: &mut egui::Ui, colors: &design::Palette, on: &mut bool) {
	let font = egui::FontId::new(12.0, design::semibold_family(ui.ctx()));
	let padding = egui::vec2(6.0, 3.0);
	let hit =
		ui.painter()
			.layout_no_wrap("@ OFF".to_owned(), font.clone(), colors.muted)
			.size() + 2.0 * padding;
	let (rect, mut response) = ui.allocate_exact_size(hit, egui::Sense::click());
	if response.clicked() {
		*on = !*on;
		response.mark_changed();
	}
	let label = if *on { "@ ON" } else { "@ OFF" };
	let color = if *on { colors.link } else { colors.muted };
	let hover = if *on {
		"Click to disable pinging the original author."
	} else {
		"Click to enable pinging the original author."
	};
	if response.hovered() || response.has_focus() {
		ui.painter().rect_filled(rect, 6, colors.hover);
	}
	let galley = ui.painter().layout_no_wrap(label.to_owned(), font, color);
	ui.painter()
		.galley(rect.center() - galley.size() / 2.0, galley, color);
	response.widget_info(|| {
		egui::WidgetInfo::selected(
			egui::Role::CheckBox,
			ui.is_enabled(),
			*on,
			crate::i18n::translate("lib-mention-switch-ping-the-original-author"),
		)
	});
	response.on_hover_text(hover);
}

enum MentionWrite {
	Inserted,
	DidNotFit,
}

impl MessagingUi {
	/// Retire secret input even when the conversation UI is no longer drawn.
	pub fn retire_mfa_input(&mut self, ctx: &egui::Context, state: &State) {
		self.mfa.retire(ctx, state);
	}

	pub fn interaction_modal_id(&self) -> Option<Id> {
		self.interaction_components.modal_id()
	}
	pub fn set_interaction_files(&mut self, custom_id: &str, files: Vec<(usize, String)>) {
		self.interaction_components.set_files(custom_id, files);
	}

	/// Feed the frame's middle button before `show`. Never fed means never pressed.
	/// A bound middle button does not also start autoscroll.
	pub fn middle_button(&mut self, middle: scroll::Middle) {
		if self.button_claimed(egui::PointerButton::Middle) {
			self.scroll.middle(scroll::Middle::default());
		} else {
			self.scroll.middle(middle);
		}
	}

	/// Feed mouse 4 / mouse 5 edge presses before `show`. Flags OR so a second feed cannot clear a press.
	pub fn side_buttons(&mut self, side: scroll::SidePress) {
		self.side_press.back |= side.back;
		self.side_press.forward |= side.forward;
	}

	fn drain_side_press(&mut self) -> scroll::SidePress {
		let press = self.side_press;
		self.side_press = scroll::SidePress::default();
		press
	}

	/// True while a drive needs the cursor position, including outside the window.
	pub fn tracking_pointer(&self) -> bool {
		self.scroll.tracking()
	}

	/// Returns whether the configured push-to-talk chord is held in the focused window.
	pub fn push_to_talk_down(&self, ctx: &egui::Context) -> bool {
		self.voice_hold_down(ctx, model::KeybindAction::PushToTalk)
	}

	/// Returns whether the configured push-to-mute chord is held in the focused window.
	pub fn push_to_mute_down(&self, ctx: &egui::Context) -> bool {
		self.voice_hold_down(ctx, model::KeybindAction::PushToMute)
	}

	/// Text entry suppresses held keys, but a held mouse button is never typing.
	fn voice_hold_down(&self, ctx: &egui::Context, action: model::KeybindAction) -> bool {
		let chord = self.keybinds.chord(action);
		if !model::keybinds::is_mouse_button(&chord.key) && ctx.egui_wants_keyboard_input() {
			return false;
		}
		ctx.input(|input| input.focused && crate::keybinds::down(input, chord))
	}

	/// Whether egui must see mouse 3–5: one is bound, or a binding is being recorded.
	/// Otherwise the host strips them for autoscroll and history navigation.
	pub fn wants_mouse_buttons(&self) -> bool {
		[
			egui::PointerButton::Middle,
			egui::PointerButton::Extra1,
			egui::PointerButton::Extra2,
		]
		.into_iter()
		.any(|button| self.button_claimed(button))
	}

	/// Whether a binding claims `button`, or a capture may, instead of its built-in role.
	fn button_claimed(&self, button: egui::PointerButton) -> bool {
		self.keybind_capture.is_some()
			|| model::KeybindAction::ALL.into_iter().any(|action| {
				crate::keybinds::button_from_name(&self.keybinds.chord(action).key) == Some(button)
			})
	}

	/// Returns focused-window mute/deafen presses that were not claimed by a native global hotkey.
	pub fn voice_toggle_pressed(&self, ctx: &egui::Context, global_mask: u8) -> u8 {
		if !ctx.input(|input| input.focused) {
			return 0;
		}
		let wants_kb = ctx.egui_wants_keyboard_input();
		ctx.input_mut(|input| {
			let mut toggles = 0;
			let mute_chord = self.keybinds.chord(model::KeybindAction::ToggleMute);
			if global_mask & 1 == 0
				&& (!wants_kb || model::keybinds::is_mouse_button(&mute_chord.key))
				&& crate::keybinds::pressed(input, mute_chord)
			{
				toggles |= 1;
			}
			let deafen_chord = self.keybinds.chord(model::KeybindAction::ToggleDeafen);
			if global_mask & 2 == 0
				&& (!wants_kb || model::keybinds::is_mouse_button(&deafen_chord.key))
				&& crate::keybinds::pressed(input, deafen_chord)
			{
				toggles |= 2;
			}
			toggles
		})
	}

	/// Pending profile picture selection: (generation, own user, editor revision).
	pub fn take_profile_picture_request(&mut self) -> Option<(u64, Id, u64)> {
		self.settings.editor.avatar_request.take()
	}
	pub fn accept_profile_picture(
		&mut self,
		ctx: &egui::Context,
		request: (u64, Id, u64),
		result: Result<Option<(String, egui::ColorImage)>, &'static str>,
	) {
		if let Err(error) = self.settings.editor.accept_avatar(ctx, request, result) {
			self.toasts.push(design::Level::Error, error);
		}
	}
	pub fn take_group_icon_request(&mut self) -> Option<(u64, Id, u64)> {
		self.group_menu.icon_request.take()
	}
	pub fn accept_group_icon(
		&mut self,
		ctx: &egui::Context,
		request: (u64, Id, u64),
		result: Result<Option<(String, egui::ColorImage)>, &'static str>,
	) {
		self.group_menu.accept_icon(ctx, request, result);
	}
	pub fn take_create_server_icon_request(&mut self) -> Option<(u64, Id, u64)> {
		self.join_server.take_icon_request()
	}
	pub fn accept_create_server_icon(
		&mut self,
		ctx: &egui::Context,
		request: (u64, Id, u64),
		result: Result<Option<(String, egui::ColorImage)>, &'static str>,
	) {
		self.join_server.accept_icon(ctx, request, result);
	}
	pub fn timeline_reflows(&self) -> (u64, u64) {
		(
			self.timeline.reflow_frames,
			self.timeline.consecutive_reflows,
		)
	}
	/// Fixture-only: keep the synthetic slash picker visible without window focus.
	#[cfg(feature = "demo")]
	pub fn preview_slash_commands(&mut self) {
		self.preview_slash_commands = true;
	}
	/// Fixture-only: show the forum list as `layout`.
	#[cfg(feature = "demo")]
	pub fn preview_forum_layout(&mut self, layout: model::forum::Layout) {
		self.forum.preview_layout(layout);
	}
	/// Fixture-only: open the settings dialog of `channel` at startup.
	#[cfg(feature = "demo")]
	pub fn preview_channel_settings(&mut self, channel: Id, generation: u64) {
		self.channel_menu.preview_settings(channel, generation);
	}
	#[cfg(feature = "demo")]
	pub fn preview_channel_creation(&mut self, channel: Id, generation: u64) {
		self.channel_menu.preview_creation(channel, generation);
	}
	/// Fixture-only: filter the forum list by `tags`, optionally with the post composer open.
	#[cfg(feature = "demo")]
	pub fn preview_forum(&mut self, forum: Id, tags: &[Id], draft: Option<&str>) {
		self.forum.preview(forum, tags, draft);
	}
	#[cfg(any(test, feature = "demo"))]
	pub fn preview_community(&mut self, state: &mut State, guild: Id, detail: bool, edit: bool) {
		self.guild = Some(guild);
		self.community_ui.preview(state, guild, detail, edit);
	}
	/// Fixture-only: open the server invite dialog at startup, as the rail menu would.
	#[cfg(any(test, feature = "demo"))]
	pub fn preview_server_invite(&mut self, state: &mut State, guild: Id) {
		if let Some(channel) = state.invite_channel(guild) {
			self.guild = Some(guild);
			self.server_menu.open_invite(state, guild, channel);
		}
	}
	/// Fixture-only: open the Threads dialog for `parent` at startup, as the header control would.
	#[cfg(any(test, feature = "demo"))]
	pub fn preview_threads(&mut self, parent: Id) {
		self.archive_parent = Some(parent);
	}
	/// Fixture-only entry point: opens People and the profile card for `user` as if clicked.
	#[cfg(any(test, feature = "demo"))]
	pub fn preview_profile(&mut self, user: model::User) {
		self.members_narrow_open = true;
		self.profile.command_open(user);
	}
	/// Fixture-only entry point: the profile popout alone, beside a docked member list.
	#[cfg(any(test, feature = "demo"))]
	pub fn preview_profile_popout(&mut self, user: model::User) {
		self.profile.command_open(user);
	}
	/// Fixture-only entry point: a profile card with its badge directory (focused on
	/// `selected`) or, without one, the account's customize menu at `anchor`.
	#[cfg(any(test, feature = "demo"))]
	pub fn preview_badges(&mut self, user: model::User, selected: Option<u32>, anchor: egui::Pos2) {
		let id = user.id;
		self.profile.command_open(user);
		self.profile.preview_badges(id, selected, anchor);
	}
	#[cfg(any(test, feature = "demo"))]
	pub fn preview_full_profile(&mut self, user: model::User) {
		self.profile.open_full(user);
	}
	/// Fixture-only entry point: opens the emoji popout as if the composer button was clicked.
	/// Fixture-only entry point: stage a synthetic attachment as if it had been selected.
	/// Repeated calls build up a batch, like choosing several files.
	#[cfg(any(test, feature = "demo"))]
	pub fn preview_attachment(
		&mut self,
		filename: &str,
		bytes: u64,
		preview: Option<egui::ColorImage>,
	) {
		if self.attachment.is_none() {
			self.attachment = Some((filename.to_owned(), bytes));
		}
		self.attachment_files.push((filename.to_owned(), bytes));
		self.attachment_previews
			.resize(self.attachment_files.len() - 1, None);
		self.attachment_previews
			.push(preview.map(std::sync::Arc::new));
	}
	/// Every selected file; older callers and tests may stage only the first one.
	fn selected_files(&self) -> Vec<(String, u64)> {
		if self.attachment_files.is_empty() {
			self.attachment.iter().cloned().collect()
		} else {
			self.attachment_files.clone()
		}
	}
	/// Textures for `attachment_previews`, uploaded once per pixel buffer and reused per frame.
	fn attachment_textures(&mut self, ctx: &egui::Context) -> Vec<Option<egui::TextureHandle>> {
		let count = self.selected_files().len();
		self.attachment_textures.resize(count, None);
		(0..count)
			.map(|index| {
				let image = self.attachment_previews.get(index).cloned().flatten();
				let Some(image) = image else {
					self.attachment_textures[index] = None;
					return None;
				};
				let key = std::sync::Arc::as_ptr(&image) as usize;
				if self.attachment_textures[index]
					.as_ref()
					.is_none_or(|(k, _)| *k != key)
				{
					let handle = ctx.load_texture(
						format!("attachment-preview-{index}"),
						egui::ImageData::Color(image),
						egui::TextureOptions::LINEAR,
					);
					self.attachment_textures[index] = Some((key, handle));
				}
				self.attachment_textures[index]
					.as_ref()
					.map(|(_, handle)| handle.clone())
			})
			.collect()
	}
	/// Moves staged files and thumbnails into the optimistic row for `command`.
	pub fn stage_pending_upload(&mut self, ctx: &egui::Context, command: &Command) {
		let files = self.selected_files();
		if let Command::Send { nonce, .. } = command
			&& !files.is_empty()
		{
			let textures = self.attachment_textures(ctx);
			self.pending_upload = Some(pending::Upload {
				nonce: nonce.clone(),
				files: files
					.into_iter()
					.zip(textures)
					.map(|((_, bytes), preview)| pending::UploadFile { bytes, preview })
					.collect(),
				progress: None,
			});
			self.attachment = None;
			self.attachment_files.clear();
			self.attachment_previews.clear();
			self.attachment_textures.clear();
			self.upload_busy = true;
		}
	}
	/// Synthetic pending state only; no command is dispatched or file uploaded.
	#[cfg(any(test, feature = "demo"))]
	pub fn preview_sending(&mut self, ctx: &egui::Context, state: &mut State) {
		if !state.demo {
			return;
		}
		let Some(channel) = state.selected else {
			return;
		};
		state
			.drafts
			.insert(channel, "Here’s the attachment — sending it now.".into());
		let files = self.selected_files();
		let names: Vec<&str> = files.iter().map(|(name, _)| name.as_str()).collect();
		if let Some(command) = state.prepare_send_with_attachments(&names) {
			self.stage_pending_upload(ctx, &command);
			if let Some(upload) = &mut self.pending_upload {
				let bytes = upload.bytes();
				upload.progress = Some((bytes * 42 / 100, bytes));
			}
		}
	}
	/// Byte counts describe data supplied to HTTP, never message delivery.
	pub fn update_upload_progress(&mut self, progress: Option<(u64, u64)>, sending: bool) {
		if let Some(upload) = &mut self.pending_upload {
			upload.progress = if sending {
				Some((upload.bytes(), upload.bytes()))
			} else {
				progress
			};
		}
	}
	/// A local Rich Presence client asked to show an invite. It is only prefilled here:
	/// the user still confirms the lookup and the join.
	pub fn open_rpc_invite(&mut self, generation: u64, code: String) {
		if invites::input_code(&code).is_some() {
			self.join_server.open_with(generation, code);
		}
	}
	/// Fixture-only: open the join-server dialog at startup.
	#[cfg(any(test, feature = "demo"))]
	pub fn preview_join_server(&mut self, generation: u64) {
		self.join_server.open(generation);
	}
	/// Fixture-only: show the native verification flow without contacting a provider.
	#[cfg(any(test, feature = "demo"))]
	pub fn preview_verification(&mut self, state: &mut State) {
		if !state.demo {
			return;
		}
		let code = "synthetic-verification".to_owned();
		state.invites.insert(
			code.clone(),
			(
				std::time::Instant::now(),
				Some(Ok(model::InvitePreview {
					guild: Id(424242),
					embed: model::Embed {
						title: Some("NuSerein community".into()),
						..Default::default()
					},
				})),
			),
		);
		state.invite_join = Default::default();
		state.invite_join.code = code;
		state.invite_join.pending = true;
		state.apply(client_core::Envelope {
			generation: state.generation,
			event: client_core::Event::InviteChallenge {
				request: 0,
				challenge: Box::new(
					client_core::captcha::Challenge::new(
						"synthetic-preview".into(),
						None,
						None,
						None,
						false,
					)
					.expect("valid synthetic challenge"),
				),
			},
		});
	}
	/// Fixture-only: open the screen-share picker for the fixture call at startup.
	#[cfg(any(test, feature = "demo"))]
	pub fn preview_screen_share(&mut self, state: &State) {
		self.screen.launch(state);
	}
	/// Fixture-only: open the pinned messages popout on the next frame.
	#[cfg(any(test, feature = "demo"))]
	pub fn preview_pins(&mut self) {
		self.search.preview_pins();
	}
	/// Fixture-only: open the account popout above the footer card at startup.
	#[cfg(any(test, feature = "demo"))]
	pub fn preview_account_menu(&mut self, generation: u64) {
		self.account_menu.preview(generation);
	}
	#[cfg(any(test, feature = "demo"))]
	pub fn preview_server_notifications(&mut self, state: &mut State, guild: Id) {
		self.guild = Some(guild);
		self.navigation_channel = state.selected;
		self.server_menu.open_notifications(state, guild);
	}
	#[cfg(any(test, feature = "demo"))]
	pub fn preview_custom_status(&mut self, generation: u64) {
		let draft = self.own_presence.custom_status.clone();
		self.account_menu.preview_editor(generation, draft);
	}
	#[cfg(any(test, feature = "demo"))]
	pub fn preview_sticker_picker(&mut self) {
		self.emoji_picker.open_stickers(None);
	}
	#[cfg(any(test, feature = "demo"))]
	pub fn preview_emoji_picker(&mut self) {
		self.emoji_picker.preview();
	}
	/// Fixture-only: open the media viewer on `attachment` of `message` at startup.
	#[cfg(any(test, feature = "demo"))]
	pub fn preview_image_viewer(&mut self, message: model::Id, attachment: model::Id) {
		self.timeline.preview_image_viewer(message, attachment);
	}
	/// Fixture-only: opens the GIFs tab at `section` (`""`, `favorites`, `trending` or a query).
	#[cfg(any(test, feature = "demo"))]
	pub fn preview_gif_picker(&mut self, section: &str) {
		self.emoji_picker.preview_gifs(section);
	}
	/// Fixture-only entry point: opens the search pane and submits `query` on the first frame.
	#[cfg(any(test, feature = "demo"))]
	pub fn preview_search(&mut self, query: &str) {
		self.search.preview(query);
	}

	pub fn downloads(&mut self) -> &mut DownloadUi {
		&mut self.timeline.download
	}
	pub fn audio(&mut self) -> &mut AudioUi {
		&mut self.timeline.audio
	}
	pub fn video(&mut self) -> &mut VideoUi {
		&mut self.timeline.video
	}
	pub fn take_voice_fullscreen_request(&mut self) -> Option<bool> {
		self.voice_fullscreen_request.take()
	}
	pub fn clear_avatars(&mut self) {
		self.avatars = avatars::Avatars::default();
	}
	pub fn take_avatar_requests(&mut self) -> Vec<String> {
		self.avatars.take_requests()
	}
	/// Next idle animation/inline texture release; see `take_avatar_requests`.
	pub fn avatar_release_at(&self) -> Option<std::time::Instant> {
		self.avatars.next_release()
	}
	pub fn accept_gif_animation(&mut self, key: String, frames: GifFrames) {
		self.avatars.accept_animation(key, frames);
	}
	pub fn accept_avatar(
		&mut self,
		ctx: &egui::Context,
		key: String,
		image: Option<egui::ColorImage>,
	) {
		self.avatars.accept(ctx, key, image);
	}
	pub fn clear(&mut self) {
		// The native window must not stay in media fullscreen after the account is gone.
		self.exit_voice_fullscreen();
		self.timeline.video.exit_fullscreen();
		let video_fullscreen_request = self.timeline.video.fullscreen_request.take();
		// Window preferences belong to the application, not the account being cleared.
		*self = Self {
			build: self.build,
			voice_fullscreen_request: self.voice_fullscreen_request.take(),
			// The switcher roster belongs to the device, not to the account being cleared.
			accounts: std::mem::take(&mut self.accounts),
			updates: std::mem::take(&mut self.updates),
			custom_font: std::mem::take(&mut self.custom_font),
			minimize_to_tray: self.minimize_to_tray,
			tray_available: self.tray_available,
			tray_status: self.tray_status,
			startup_enabled: self.startup_enabled,
			startup_minimized: self.startup_minimized,
			startup_available: self.startup_available,
			startup_busy: self.startup_busy,
			startup_status: self.startup_status,
			startup_disable_requested: self.startup_disable_requested,
			..Self::default()
		};
		self.timeline.video.fullscreen_request = video_fullscreen_request;
	}
	pub fn preview_mod_view(&mut self, state: &mut State, guild: Id, user: Id) -> Option<Command> {
		self.mod_view.open(state, guild, user)
	}
	pub fn restore_channel_preferences(&mut self, preferences: model::ChannelPreferences) {
		if !self.channel_preferences_changed && preferences.is_valid() {
			self.channel_preferences = preferences;
			self.channel_preferences_loaded = true;
			self.channel_preferences_status = "";
			self.last_channels_seed = true;
			self.last_channel_recorded = None;
			self.channel_cache.invalidate();
		}
	}
	/// Direct messages and servers each remember whether the wide member list is open.
	fn members_preference(&mut self, state: &State) -> &mut bool {
		if state
			.selected
			.and_then(|id| state.channel(id))
			.is_some_and(|channel| channel.guild.is_none())
		{
			&mut self.reading_preferences.show_members_dms
		} else {
			&mut self.reading_preferences.show_members
		}
	}
	/// Servers reopen to their last text channel across restarts, not only within a session.
	fn sync_last_channels(&mut self, state: &mut State) {
		if !self.channel_preferences_loaded {
			return;
		}
		if std::mem::take(&mut self.last_channels_seed) {
			state.seed_viewed_channels(&self.channel_preferences.last_channels);
		}
		if state.selected == self.last_channel_recorded {
			return;
		}
		self.last_channel_recorded = state.selected;
		if let Some(channel) = state
			.selected
			.and_then(|id| state.channel(id))
			.filter(|channel| !matches!(channel.kind, 2 | 10..=13))
			&& let Some(guild) = channel.guild
		{
			self.channel_preferences_changed |=
				self.channel_preferences.remember_channel(guild, channel.id);
		}
	}
	/// Native alerts retain their exact target and use the normal permission-checked navigation.
	pub fn open_notification(
		&mut self,
		state: &mut State,
		channel: Id,
		message: Option<Id>,
	) -> Option<Command> {
		if self.server_settings.is_open() {
			self.toasts.push(
				design::Level::Info,
				"Close server settings to open this notification.",
			);
			return None;
		}
		let guild = state.channel(channel)?.guild;
		match state.open_chat_link(guild, channel, message) {
			Ok(command) => {
				self.settings.open = false;
				self.guild = guild;
				command
			}
			Err(status) => {
				state.status = status;
				None
			}
		}
	}
	fn navigate_history(&mut self, state: &mut State, commands: &mut Vec<Command>, back: bool) {
		if self.timeline.video.is_fullscreen()
			|| self.is_voice_fullscreen()
			|| self.channel_menu.is_open()
		{
			return;
		}
		if self.settings.open {
			if back {
				self.settings.open = false;
			}
		} else if self.server_settings.is_open() {
			if back {
				let _ = self.server_settings.navigate_away(state);
			}
		} else if let Some(NavStep {
			command: Some(command),
		}) = if back {
			state.navigate_back()
		} else {
			state.navigate_forward()
		} {
			commands.push(command);
		}
	}

	fn shows_title_bar(&self) -> bool {
		!cfg!(target_os = "linux") && !self.hide_title_bar
	}
	fn drain_profile(&mut self, state: &mut State, commands: &mut Vec<Command>) -> bool {
		let mut cleared = false;
		for effect in self.profile.drain_effects() {
			match effect {
				profiles::ProfileEffect::ClearCore => {
					commands.push(state.clear_profile());
					self.profile_link = None;
					cleared = true;
				}
			}
		}
		cleared
	}

	/// The server a profile card opens in: the one being administered, else the open channel's.
	fn profile_guild(&self, state: &State) -> Option<model::Id> {
		self.server_settings.guild().or_else(|| {
			state
				.selected
				.and_then(|id| state.channel(id))
				.and_then(|channel| channel.guild)
		})
	}

	fn sync_profile(
		&mut self,
		state: &mut State,
		commands: &mut Vec<Command>,
		user: &model::User,
		profile_guild: Option<model::Id>,
	) {
		if user.webhook {
			if state.profile.is_some() {
				commands.push(state.clear_profile());
			}
			return;
		}
		if state
			.profile
			.as_ref()
			.is_some_and(|view| view.user == user.id && view.guild == profile_guild)
		{
			return;
		}
		self.profile_link = None;
		#[cfg(any(test, feature = "demo"))]
		if state.demo {
			state.profile = Some(client_core::profile::ProfileView {
				user: user.id,
				guild: profile_guild,
				request: 0,
				loading: false,
				error: None,
				data: Some(
					state
						.own_profile
						.data
						.as_ref()
						.filter(|data| data.user.id == user.id)
						.cloned()
						.unwrap_or_else(|| profiles::synthetic(user, profile_guild)),
				),
			});
		}
		if !state.demo
			&& let Some(command) = state.request_profile(user.id, profile_guild)
		{
			commands.push(command);
		}
	}

	fn toggle_gif_favorite(&mut self, ctx: &egui::Context, state: &mut State, gif: &model::Gif) {
		if !state.toggle_gif_favorite(gif) {
			let message = if state.gifs.sync_pending.is_some() {
				"gif-favorites-sync-pending"
			} else {
				"gif-favorites-toggle-failed"
			};
			self.toasts
				.push(design::Level::Warning, crate::i18n::translate(message));
		}
		ctx.request_repaint();
	}
}

#[cfg(test)]
mod gif_favorite_tests;

#[cfg(test)]
mod composer_tests {
	use super::*;
	use client_core::MAX_CONTENT;

	#[test]
	fn notification_navigation_targets_the_message_and_updates_the_sidebar_without_acknowledging() {
		let mut state = test_support::chat_demo_state();
		let origin = state.selected.unwrap();
		let target = state.timeline.iter().next().unwrap().id;
		state
			.drafts
			.insert(origin, "Preserve notification draft".into());
		let mut view = MessagingUi::default();
		assert!(
			view.open_notification(&mut state, origin, Some(target))
				.is_none()
		);
		assert_eq!(state.search_target, Some(target));
		assert_eq!(view.guild, state.channel(origin).unwrap().guild);
		let direct = state
			.channels
			.iter()
			.find(|channel| channel.kind == 1)
			.unwrap()
			.id;
		let command = view
			.open_notification(&mut state, direct, Some(target))
			.unwrap();
		assert!(
			matches!(command, Command::HistoryAround { channel, target: requested, .. } if channel == direct && requested == target)
		);
		assert_eq!(state.selected, Some(direct));
		assert_eq!(view.guild, None);
		assert_eq!(state.drafts[&origin], "Preserve notification draft");
		assert!(
			view.open_notification(&mut state, Id(0), Some(target))
				.is_none()
		);
		assert_eq!(state.selected, Some(direct));
	}

	#[test]
	fn unread_shortcuts_preserve_drafts_and_obey_settings_ime_and_capture() {
		for blocked in 0..4 {
			let mut state = test_support::chat_demo_state();
			let previous = state.selected.unwrap();
			state.drafts.insert(previous, "Unsent draft".into());
			let mut channel = state.channels.iter().find(|c| c.kind == 1).unwrap().clone();
			channel.id = Id(50001);
			channel.last_message = Some(Id(50005));
			state.channels.push(channel);
			state.invalidate_navigation();
			state.apply(client_core::Envelope {
				generation: state.generation,
				event: client_core::Event::ReadState(client_core::read_state::Event::Snapshot {
					entries: Some(
						state
							.channels
							.iter()
							.filter_map(|c| {
								c.last_message.map(|latest| {
									(
										c.id,
										Some(if c.id == Id(50001) { Id(50004) } else { latest }),
										0,
									)
								})
							})
							.collect(),
					),
					version: None,
					partial: false,
				}),
			});
			let mut view = MessagingUi {
				guild: state.channel(previous).and_then(|c| c.guild),
				..Default::default()
			};
			if blocked == 1 {
				view.settings.open = true;
			}
			if blocked == 2 {
				view.ime_active = true;
			}
			if blocked == 3 {
				view.keybind_capture = Some(model::KeybindAction::SwitchConversation);
			}
			let mut events = vec![egui::Event::Key {
				key: egui::Key::ArrowDown,
				physical_key: None,
				pressed: true,
				repeat: false,
				modifiers: egui::Modifiers {
					alt: true,
					shift: true,
					..Default::default()
				},
			}];
			if blocked == 2 {
				events.push(egui::Event::Ime(egui::ImeEvent::Preedit {
					text: "未確定".into(),
					active_range_chars: None,
				}));
			}
			let ctx = egui::Context::default();
			design::apply(&ctx);
			let mut commands = Vec::new();
			ctx.run_ui(
				egui::RawInput {
					screen_rect: Some(egui::Rect::from_min_size(
						egui::Pos2::ZERO,
						egui::vec2(1120.0, 760.0),
					)),
					events,
					focused: true,
					..Default::default()
				},
				|ui| commands.extend(view.show(ui, &mut state)),
			)
			.drop_without_applying_deltas();
			assert_eq!(
				state.selected,
				Some(if blocked == 0 { Id(50001) } else { previous })
			);
			assert!(
				state.drafts[&previous].starts_with("Unsent draft"),
				"IME composition and navigation retain the existing draft"
			);
			assert!(
				!commands.iter().any(|command| matches!(
					command,
					Command::MarkRead { .. }
						| Command::MarkAllRead { .. }
						| Command::MarkGuildRead { .. }
						| Command::Voice(_)
				)),
				"navigation never acknowledges or opens a call"
			);
		}
	}

	#[test]
	fn next_message_tray_can_remove_files_while_the_previous_message_uploads() {
		for attach_busy in [false, true] {
			let ctx = egui::Context::default();
			design::apply(&ctx);
			let mut view = MessagingUi {
				attachment_files: vec![("next.txt".into(), 32), ("keep.txt".into(), 64)],
				upload_busy: true,
				attach_busy,
				..Default::default()
			};
			let frame = |view: &mut MessagingUi, events| {
				let output = ctx.run_ui(
					egui::RawInput {
						screen_rect: Some(egui::Rect::from_min_size(
							egui::Pos2::ZERO,
							egui::vec2(600.0, 300.0),
						)),
						events,
						focused: true,
						..Default::default()
					},
					|ui| view.attachment_tray(ui, false, 500_000_000),
				);
				let remove = output
					.shapes
					.iter()
					.find_map(|shape| match &shape.shape {
						egui::Shape::Rect(rect) if rect.rect.size() == egui::vec2(36.0, 36.0) => {
							Some(rect.rect.center())
						}
						_ => None,
					})
					.expect("first attachment's Remove button");
				output.drop_without_applying_deltas();
				remove
			};
			for _ in 0..3 {
				frame(&mut view, vec![]);
			}
			let remove = frame(&mut view, vec![]);
			for pressed in [true, false] {
				frame(
					&mut view,
					vec![
						egui::Event::PointerMoved(remove),
						egui::Event::PointerButton {
							pos: remove,
							button: egui::PointerButton::Primary,
							pressed,
							modifiers: egui::Modifiers::NONE,
						},
					],
				);
			}
			assert_eq!(view.remove_attachment_index, (!attach_busy).then_some(0));
			assert!(!view.cancel_upload_requested);
			assert!(!view.remove_attachment_requested);
		}
	}

	#[test]
	fn header_search_collapses_and_keeps_keyboard_and_disabled_behavior() {
		let key = |key| egui::Event::Key {
			key,
			physical_key: None,
			pressed: true,
			repeat: false,
			modifiers: egui::Modifiers::NONE,
		};
		for (width, expected_width) in [(260.0, 32.0), (600.0, 144.0)] {
			for enabled in [false, true] {
				let ctx = egui::Context::default();
				design::apply(&ctx);
				let frame = |events| {
					let mut search = None;
					let output = ctx.run_ui(
						egui::RawInput {
							screen_rect: Some(egui::Rect::from_min_size(
								egui::Pos2::ZERO,
								egui::vec2(width, 200.0),
							)),
							events,
							focused: true,
							..Default::default()
						},
						|ui| {
							search = Some(header_search_button(ui, enabled, "Search"));
							let _ = ui.button("Next control");
						},
					);
					output.drop_without_applying_deltas();
					search.unwrap()
				};
				for _ in 0..3 {
					frame(vec![]);
				}
				let response = frame(vec![key(egui::Key::Tab)]);
				assert_eq!(response.rect.width(), expected_width);
				assert_eq!(ctx.memory(|memory| memory.has_focus(response.id)), enabled);
				assert_eq!(frame(vec![key(egui::Key::Enter)]).clicked(), enabled);
			}
		}
	}

	#[test]
	fn narrow_composer_context_keeps_long_author_and_edit_controls_reachable() {
		use crate::test_ui::logical_text as collect;
		let key = |key| egui::Event::Key {
			key,
			physical_key: None,
			pressed: true,
			repeat: false,
			modifiers: egui::Modifiers::NONE,
		};
		for width in [280.0, 360.0] {
			for mode in 0..3 {
				let ctx = egui::Context::default();
				design::apply(&ctx);
				let mut state = edit_state();
				let mut source = state.timeline.get(Id(20)).unwrap().clone();
				source.author.name = "Very long synthetic author 漢字 👋 ".repeat(8);
				state.timeline.insert(source, true, false).unwrap();
				let mut view = MessagingUi::default();
				if mode == 0 {
					state.reply = Some(client_core::Reply::to(Id(20)));
				} else {
					view.editing = Some((Id(10), Id(20), "Changed unsent text".into()));
					view.edit_modified = Some((Id(10), Id(20), true));
					view.composer_edit = Some((Id(10), Id(20)));
					view.edit_sent = (mode == 1).then(|| SubmittedEdit {
						channel: Id(10),
						message: Id(20),
						draft: "Changed unsent text".into(),
						content: "Changed unsent text".into(),
					});
					if mode == 2 {
						state.timeline.delete(Id(20)).unwrap();
					}
				}
				let draft = state.drafts.clone();
				let frame = |view: &mut MessagingUi, state: &mut State, events| {
					let mut labels = vec![];
					let mut commands = vec![];
					let output = ctx.run_ui(
						egui::RawInput {
							screen_rect: Some(egui::Rect::from_min_size(
								egui::Pos2::ZERO,
								egui::vec2(width, 350.0),
							)),
							events,
							focused: true,
							..Default::default()
						},
						|ui| {
							view.composer(ui, state, Id(10), &ctx, &mut commands);
							assert!(
								ui.min_rect().bottom() <= ui.clip_rect().bottom(),
								"the context strip must leave the message input inside the viewport"
							);
						},
					);
					assert!(commands.is_empty());
					for shape in &output.shapes {
						collect(&shape.shape, &mut labels);
					}
					for event in &output.platform_output.events {
						if let egui::output::OutputEvent::FocusGained(info) = event
							&& let Some(label) = &info.label
							&& let Some(response) = ctx
								.memory(|memory| memory.focused())
								.and_then(|id| ctx.read_response(id))
						{
							labels.push((label.clone(), response.rect));
						}
					}
					output.drop_without_applying_deltas();
					labels
				};
				for _ in 0..3 {
					frame(&mut view, &mut state, vec![]);
				}
				let labels = frame(&mut view, &mut state, vec![]);
				let context = labels
					.iter()
					.find(|(label, _)| match mode {
						0 => label.starts_with("Replying to"),
						1 => label.starts_with("Editing message"),
						_ => label.starts_with("Message unavailable"),
					})
					.expect("composer context remains present");
				assert!(context.1.left() >= 0.0 && context.1.right() <= width);
				assert!(context.1.height() < 24.0, "context must stay on one line");
				let action = if mode == 0 {
					"View original"
				} else if mode == 2 {
					"Copy edit text"
				} else {
					""
				};
				if !action.is_empty() {
					let rect = labels.iter().find(|(label, _)| label == action).unwrap().1;
					assert!(rect.left() >= 0.0 && rect.right() <= width);
				}
				ctx.memory_mut(|memory| {
					if let Some(id) = memory.focused() {
						memory.surrender_focus(id);
					}
				});
				frame(&mut view, &mut state, vec![egui::Event::PointerGone]);
				let cancel = if mode == 0 {
					"Cancel reply"
				} else {
					"Cancel edit"
				};
				let mut cancelled = false;
				for _ in 0..16 {
					let labels = frame(&mut view, &mut state, vec![key(egui::Key::Tab)]);
					if let Some((_, rect)) = labels.iter().find(|(label, _)| label == cancel) {
						assert!(rect.left() >= 0.0 && rect.right() <= width);
						frame(&mut view, &mut state, vec![key(egui::Key::Enter)]);
						cancelled = true;
						break;
					}
				}
				assert!(cancelled, "Tab must reach the visible cancel control");
				assert!(if mode == 0 {
					state.reply.is_none()
				} else {
					view.editing.is_none()
				});
				assert_eq!(state.drafts, draft);
			}
		}
	}

	#[test]
	fn composer_context_menu_saves_cuts_and_gates_clipboard_requests() {
		let ctx = egui::Context::default();
		let mut state = edit_state();
		let frame = |view: &mut MessagingUi, state: &mut State, events| {
			let output = ctx.run_ui(
				egui::RawInput {
					screen_rect: Some(egui::Rect::from_min_size(
						egui::Pos2::ZERO,
						egui::vec2(1000.0, 700.0),
					)),
					events,
					focused: true,
					..Default::default()
				},
				|ui| {
					view.show(ui, state);
				},
			);
			let mut labels = Vec::new();
			fn walk(shape: &egui::Shape, labels: &mut Vec<(String, egui::Rect)>) {
				match shape {
					egui::Shape::Text(text) => labels.push((
						text.galley.job.text.clone(),
						text.galley.rect.translate(text.pos.to_vec2()),
					)),
					egui::Shape::Vec(shapes) => shapes.iter().for_each(|shape| walk(shape, labels)),
					_ => {}
				}
			}
			output
				.shapes
				.iter()
				.for_each(|shape| walk(&shape.shape, &mut labels));
			output.drop_without_applying_deltas();
			labels
		};
		let click = |pos, button| {
			let mut events = vec![egui::Event::PointerMoved(pos)];
			for pressed in [true, false] {
				events.push(egui::Event::PointerButton {
					pos,
					button,
					pressed,
					modifiers: Default::default(),
				});
			}
			events
		};
		let channel = state.selected.unwrap();
		for scope_change in 0..3 {
			state = edit_state();
			let mut view = MessagingUi::default();
			for _ in 0..3 {
				frame(&mut view, &mut state, vec![]);
			}
			state.drafts.insert(channel, "draft".into());
			let composer = ctx
				.memory(|memory| memory.focused())
				.expect("composer focused");
			let rect = ctx.read_response(composer).unwrap().rect;
			frame(
				&mut view,
				&mut state,
				click(rect.center(), egui::PointerButton::Secondary),
			);
			let labels = frame(&mut view, &mut state, vec![]);
			for item in ["Cut", "Copy", "Select All"] {
				assert!(
					labels.iter().any(|(text, _)| text == item),
					"{item}: {labels:?}"
				);
			}
			let (_, paste) = labels
				.iter()
				.find(|(text, _)| text == "Paste")
				.unwrap()
				.clone();
			frame(
				&mut view,
				&mut state,
				click(paste.center(), egui::PointerButton::Primary),
			);
			if scope_change == 1 {
				state.generation += 1;
			}
			if scope_change == 2 {
				state.selected = None;
			}
			frame(&mut view, &mut state, vec![]);
			if scope_change != 0 {
				assert!(view.menu_paste.is_none());
				assert!(
					view.attachment_paste_requested.is_none(),
					"A menu click cannot start a read in a new account or conversation"
				);
				assert_eq!(
					state.status,
					"Paste cancelled; focus the message and paste again"
				);
				assert_eq!(state.drafts[&channel], "draft");
				continue;
			}
			let request = view
				.attachment_paste_requested
				.take()
				.expect("menu paste reads the system clipboard");
			assert_eq!(request.target.widget, composer);
			assert!(request.text.is_none() && request.image.is_none());
		}
		for editing in [false, true] {
			state = edit_state();
			state.drafts.insert(channel, "draft".into());
			let mut view = MessagingUi::default();
			if editing {
				view.editing = Some((channel, Id(20), "Original".into()));
			}
			for _ in 0..3 {
				frame(&mut view, &mut state, vec![]);
			}
			let composer = ctx.memory(|memory| memory.focused()).unwrap();
			let rect = ctx.read_response(composer).unwrap().rect;
			composer_selection(&ctx, composer, 0, 2);
			frame(
				&mut view,
				&mut state,
				click(
					rect.left_center() + egui::vec2(1.0, 0.0),
					egui::PointerButton::Secondary,
				),
			);
			let labels = frame(&mut view, &mut state, vec![]);
			let (_, copy) = labels.iter().find(|(text, _)| text == "Copy").unwrap();
			frame(
				&mut view,
				&mut state,
				click(copy.center(), egui::PointerButton::Primary),
			);
			assert_eq!(state.drafts[&channel], "draft");
			if editing {
				assert_eq!(view.editing.as_ref().unwrap().2, "Original");
			}
			frame(&mut view, &mut state, vec![egui::Event::Text(" ".into())]);
			assert!(ctx.memory(|memory| memory.has_focus(composer)));
			if editing {
				assert_eq!(
					view.editing.as_ref().unwrap().2,
					" iginal",
					"A space immediately after Copy replaces the preserved edit selection"
				);
				assert_eq!(state.drafts[&channel], "draft");
			} else {
				assert_eq!(
					state.drafts[&channel], " aft",
					"A space immediately after Copy replaces the preserved draft selection"
				);
			}
		}
		for (editing, cut_all, arrow_after_cut) in [
			(false, false, false),
			(false, true, false),
			(true, false, false),
			(false, false, true),
		] {
			state = edit_state();
			let mut view = MessagingUi::default();
			state.drafts.insert(channel, "draft".into());
			if editing {
				view.editing = Some((Id(10), Id(20), "Original".into()));
			}
			for _ in 0..3 {
				frame(&mut view, &mut state, vec![]);
			}
			let composer = ctx.memory(|memory| memory.focused()).unwrap();
			let rect = ctx.read_response(composer).unwrap().rect;
			let before_cut = if editing { "Original!" } else { "draft!" };
			composer_selection(
				&ctx,
				composer,
				before_cut.chars().count() - 1,
				before_cut.chars().count() - 1,
			);
			frame(&mut view, &mut state, vec![egui::Event::Text("!".into())]);
			let mut cursor = egui::TextEdit::load_state(&ctx, composer).unwrap();
			cursor
				.cursor
				.set_char_range(Some(egui::text::CCursorRange::two(
					egui::text::CCursor::new(0),
					egui::text::CCursor::new(if cut_all { 6 } else { 2 }),
				)));
			cursor.store(&ctx, composer);
			let mut open_menu = click(
				rect.left_center() + egui::vec2(1.0, 0.0),
				egui::PointerButton::Secondary,
			);
			if cut_all {
				let release = open_menu.pop().unwrap();
				frame(&mut view, &mut state, open_menu);
				frame(&mut view, &mut state, vec![release]);
			} else {
				frame(&mut view, &mut state, open_menu);
			}
			frame(&mut view, &mut state, vec![]);
			frame(&mut view, &mut state, vec![]);
			let labels = frame(&mut view, &mut state, vec![]);
			assert!(
				!egui::TextEdit::load_state(&ctx, composer)
					.unwrap()
					.cursor
					.char_range()
					.unwrap()
					.is_empty(),
				"selection for cut: {editing}/{cut_all}"
			);
			let (_, cut) = labels.iter().find(|(text, _)| text == "Cut").unwrap();
			view.draft_changes.clear();
			frame(
				&mut view,
				&mut state,
				click(cut.center(), egui::PointerButton::Primary),
			);
			let changes = std::mem::take(&mut view.draft_changes);
			if arrow_after_cut {
				frame(
					&mut view,
					&mut state,
					vec![
						edit_key(egui::Key::ArrowRight),
						egui::Event::Text("?".into()),
					],
				);
				assert_eq!(
					state.drafts[&channel], "a?ft!",
					"Arrow and typing immediately after Cut belong to the editor"
				);
				assert!(ctx.memory(|memory| memory.has_focus(composer)));
				frame(
					&mut view,
					&mut state,
					vec![composer_shortcut(egui::Key::Z, false, true)],
				);
				assert_eq!(
					state.drafts[&channel], "aft!",
					"Undo of new typing keeps the Cut as a separate step"
				);
			}
			let focus_after_cut = ctx.memory(|memory| memory.focused());
			let undo_after_cut = egui::TextEdit::load_state(&ctx, composer).unwrap().undoer();
			frame(
				&mut view,
				&mut state,
				vec![composer_shortcut(egui::Key::Z, false, true)],
			);
			assert_eq!(
				if editing {
					&view.editing.as_ref().unwrap().2
				} else {
					&state.drafts[&channel]
				},
				before_cut,
				"Cut undo must keep text typed immediately before opening the menu; focus after Cut={focus_after_cut:?}, focus after Undo={:?}, undo after Cut={undo_after_cut:?}",
				ctx.memory(|memory| memory.focused())
			);
			frame(
				&mut view,
				&mut state,
				vec![composer_shortcut(egui::Key::Z, true, true)],
			);
			view.draft_changes = changes;
			if editing {
				assert_eq!(view.editing.as_ref().unwrap().2, "iginal!");
				assert_eq!(view.edit_modified, Some((Id(10), Id(20), true)));
				assert!(view.draft_changes.is_empty());
				assert_eq!(state.drafts.get(&channel).unwrap(), "draft");
				frame(&mut view, &mut state, vec![]);
				frame(
					&mut view,
					&mut state,
					click(rect.center(), egui::PointerButton::Secondary),
				);
				let labels = frame(&mut view, &mut state, vec![]);
				let (_, paste) = labels.iter().find(|(text, _)| text == "Paste").unwrap();
				frame(
					&mut view,
					&mut state,
					click(paste.center(), egui::PointerButton::Primary),
				);
				frame(&mut view, &mut state, vec![]);
				assert!(view.menu_paste.is_none());
				let request = view
					.attachment_paste_requested
					.take()
					.expect("edit menu reads text");
				let expected = request.target.replacement("replacement");
				view.upload_busy = false;
				view.pasted_text = Some((request.target, "replacement".into()));
				frame(&mut view, &mut state, vec![]);
				assert_eq!(view.editing.as_ref().unwrap().2, expected);
				assert_eq!(state.drafts[&channel], "draft");
			} else {
				assert_eq!(
					view.draft_changes,
					[channel],
					"cut={cut_all}, draft={:?}",
					state.drafts
				);
				if cut_all {
					assert!(!state.drafts.contains_key(&channel));
				} else {
					assert_eq!(state.drafts.get(&channel).unwrap(), "aft!");
				}
			}
		}
	}

	#[test]
	fn paste_into_an_idle_conversation_focuses_composer_without_sending() {
		let ctx = egui::Context::default();
		let mut state = edit_state();
		let mut view = MessagingUi::default();
		let frame = |view: &mut MessagingUi, state: &mut State, events| {
			let mut commands = Vec::new();
			let output = ctx.run_ui(
				egui::RawInput {
					screen_rect: Some(egui::Rect::from_min_size(
						egui::Pos2::ZERO,
						egui::vec2(1000.0, 700.0),
					)),
					events,
					focused: true,
					..Default::default()
				},
				|ui| {
					commands = view.show(ui, state);
				},
			);
			output.drop_without_applying_deltas();
			commands
		};
		for _ in 0..3 {
			frame(&mut view, &mut state, vec![]);
		}
		ctx.memory_mut(|memory| {
			if let Some(id) = memory.focused() {
				memory.surrender_focus(id);
			}
		});
		let channel = state.selected.unwrap();
		state.drafts.remove(&channel);
		let commands = frame(
			&mut view,
			&mut state,
			vec![egui::Event::Paste("Pasted offline text".into())],
		);
		let composer = ctx
			.memory(|memory| memory.focused())
			.expect("paste focuses composer");
		assert!(ctx.read_response(composer).is_some());
		assert!(view.attachment_paste_requested.is_none());
		assert!(!view.upload_busy);
		assert_eq!(
			state.drafts.get(&channel).map(String::as_str),
			Some("Pasted offline text")
		);
		assert!(
			!commands
				.iter()
				.any(|command| matches!(command, Command::Send { .. } | Command::Edit { .. }))
		);
	}

	#[test]
	fn diagnostics_shortcut_copies_existing_report_and_respects_key_capture() {
		let ctx = egui::Context::default();
		let mut state = test_support::demo_state();
		let mut view = MessagingUi::default();
		let event = || egui::Event::Key {
			key: egui::Key::F12,
			physical_key: None,
			pressed: true,
			repeat: false,
			modifiers: egui::Modifiers::NONE,
		};
		view.connection_diagnostics =
			"0 ms: server requested reconnect; attempt 1\n12 ms: resumed".into();
		assert!(
			view.diagnostic_info(&ctx)
				.contains("Connection history (this session):")
		);
		assert!(
			view.diagnostic_info(&ctx)
				.contains("server requested reconnect")
		);
		for (assigned, capturing, composing, copies) in [
			(false, false, false, false),
			(true, false, false, true),
			(true, true, false, false),
			(true, false, true, false),
		] {
			view.keybinds.copy_issue_diagnostics =
				model::KeyChord::new(if assigned { "F12" } else { "" }, 0);
			view.keybind_capture = capturing.then_some(model::KeybindAction::ToggleMute);
			view.ime_active = composing;
			let expected = view.diagnostic_info(&ctx);
			let output = ctx.run_ui(
				egui::RawInput {
					focused: true,
					events: vec![
						egui::Event::Key {
							key: egui::Key::F12,
							physical_key: None,
							pressed: false,
							repeat: false,
							modifiers: egui::Modifiers::NONE,
						},
						event(),
					],
					..Default::default()
				},
				|ui| {
					assert!(!view.show(ui, &mut state).iter().any(|command| matches!(
						command,
						Command::Send { .. } | Command::Edit { .. }
					)));
				},
			);
			assert_eq!(
				output.platform_output.commands.iter().any(
					|command| matches!(command, egui::OutputCommand::CopyText(text) if text == &expected)
				),
				copies
			);
			output.drop_without_applying_deltas();
		}
	}

	#[test]
	fn a_bound_mouse_button_holds_push_to_mute_instead_of_navigating() {
		let mut view = MessagingUi::default();
		assert!(!view.wants_mouse_buttons());
		assert!(!view.button_claimed(egui::PointerButton::Middle));
		view.keybinds.push_to_mute = model::KeyChord::new("MouseExtra1", 0);
		assert!(view.wants_mouse_buttons());
		assert!(view.button_claimed(egui::PointerButton::Extra1));
		assert!(!view.button_claimed(egui::PointerButton::Extra2));
		let ctx = egui::Context::default();
		for focused in [true, false] {
			ctx.run_ui(
				egui::RawInput {
					focused,
					events: vec![egui::Event::PointerButton {
						pos: egui::pos2(10.0, 10.0),
						button: egui::PointerButton::Extra1,
						pressed: true,
						modifiers: egui::Modifiers::NONE,
					}],
					..Default::default()
				},
				|ui| {
					assert_eq!(view.push_to_mute_down(ui.ctx()), focused);
					assert!(!view.push_to_talk_down(ui.ctx()));
				},
			)
			.drop_without_applying_deltas();
		}
	}

	#[test]
	fn an_unmodified_diagnostics_key_does_not_replace_the_clipboard_while_typing() {
		let ctx = egui::Context::default();
		let mut state = edit_state();
		state.demo = true;
		let mut view = MessagingUi::default();
		view.keybinds.copy_issue_diagnostics = model::KeyChord::new("C", 0);
		for events in [
			vec![],
			vec![egui::Event::Text("seed".into())],
			vec![
				egui::Event::Key {
					key: egui::Key::C,
					physical_key: None,
					pressed: true,
					repeat: false,
					modifiers: egui::Modifiers::NONE,
				},
				egui::Event::Text("c".into()),
			],
		] {
			let output = ctx.run_ui(
				egui::RawInput {
					focused: true,
					events,
					screen_rect: Some(egui::Rect::from_min_size(
						egui::Pos2::ZERO,
						egui::vec2(900.0, 600.0),
					)),
					..Default::default()
				},
				|ui| {
					view.show(ui, &mut state);
				},
			);
			assert!(
				!output
					.platform_output
					.commands
					.iter()
					.any(|command| matches!(command, egui::OutputCommand::CopyText(_)))
			);
			output.drop_without_applying_deltas();
		}
		assert!(
			state
				.selected
				.and_then(|channel| state.drafts.get(&channel))
				.is_some_and(|draft| draft.ends_with('c')),
			"the diagnostic shortcut leaves normal typing intact"
		);
	}

	#[test]
	fn public_host_button_stays_below_attachment_card() {
		let ctx = egui::Context::default();
		let mut view = MessagingUi::default();
		view.preview_attachment("synthetic.pdf", 100, None);
		for _ in 0..2 {
			let output = ctx.run_ui(Default::default(), |ui| view.attachment_tray(ui, true, 0));
			let text_rect = |label: &str| {
				output
					.shapes
					.iter()
					.find_map(|shape| match &shape.shape {
						egui::Shape::Text(text) if text.galley.text() == label => {
							Some(egui::Rect::from_min_size(text.pos, text.galley.size()))
						}
						_ => None,
					})
					.unwrap()
			};
			assert!(text_rect("Share link instead…").top() > text_rect("synthetic.pdf").bottom());
			output.drop_without_applying_deltas();
		}
	}

	#[test]
	fn download_cancel_remains_visible_without_a_text_composer() {
		for selection in [None, Some(Id(10))] {
			let mut state = edit_state();
			state.demo = true;
			state.selected = selection;
			state.channels[0].kind = 2;
			let mut messaging = MessagingUi::default();
			messaging.downloads().active = true;
			messaging.downloads().status = "Saving synthetic attachment".into();
			let ctx = egui::Context::default();
			let output = ctx.run_ui(egui::RawInput::default(), |ui| {
				messaging.show(ui, &mut state);
			});
			assert!(output.shapes.iter().any(|shape| {
				matches!(&shape.shape, egui::Shape::Text(text) if text.galley.job.text == "Cancel download")
			}));
			output.drop_without_applying_deltas();
		}
	}

	#[test]
	fn emoticon_conversion_composer_on_off_and_rejected_sends_keep_raw_drafts() {
		for enabled in [false, true] {
			for rejection in [None, Some("budget"), Some("oversized")] {
				let mut state = edit_state();
				if rejection == Some("budget") {
					for _ in 0..64 {
						state.drafts.insert(Id(10), "Queued".into());
						assert!(state.prepare_send().is_some());
					}
				}
				state.drafts.insert(Id(10), "Hello :)".into());
				let mut view = MessagingUi {
					convert_emoticons: enabled,
					focus_switched_composer: true,
					attachment: (rejection == Some("oversized"))
						.then(|| ("large.txt".into(), 500 * 1024 * 1024 + 1)),
					..Default::default()
				};
				let ctx = egui::Context::default();
				for _ in 0..2 {
					edit_frame(&ctx, &mut view, &mut state, vec![]);
				}
				let commands = edit_frame(
					&ctx,
					&mut view,
					&mut state,
					vec![edit_key(egui::Key::Enter)],
				);
				if rejection.is_some() {
					assert!(
						!commands
							.iter()
							.any(|command| matches!(command, Command::Send { .. }))
					);
					assert_eq!(state.drafts[&Id(10)], "Hello :)");
				} else {
					let expected = if enabled { "Hello 🙂" } else { "Hello :)" };
					assert!(commands.iter().any(
						|command| matches!(command, Command::Send { content, .. } if content == expected)
					));
				}
			}
		}
	}

	#[test]
	fn emoticon_conversion_edits_wait_for_confirmation_and_keep_raw_text_on_failure() {
		for success in [false, true] {
			let mut state = edit_state();
			let mut view = MessagingUi {
				convert_emoticons: true,
				editing: Some((Id(10), Id(20), "Hello :)".into())),
				..Default::default()
			};
			let ctx = egui::Context::default();
			edit_frame(&ctx, &mut view, &mut state, vec![]);
			let commands = edit_frame(
				&ctx,
				&mut view,
				&mut state,
				vec![edit_key(egui::Key::Enter)],
			);
			let [
				Command::Edit {
					content, request, ..
				},
			] = commands.as_slice()
			else {
				panic!("One converted edit")
			};
			assert_eq!(content, "Hello 🙂");
			assert_eq!(view.editing.as_ref().unwrap().2, "Hello :)");
			assert_eq!(state.timeline.get(Id(20)).unwrap().content, "Hello 🙂");
			// An optimistic update and an unrelated result must not close the editor.
			let confirmed = state.timeline.get(Id(20)).unwrap().clone();
			state.apply_edit_result(Id(10), Id(20), request + 1, Ok(confirmed.clone()));
			edit_frame(&ctx, &mut view, &mut state, vec![]);
			assert!(view.has_edit());
			// Navigation must retain the submitted value and the raw draft.
			state.selected = Some(Id(11));
			edit_frame(&ctx, &mut view, &mut state, vec![]);
			state.selected = Some(Id(10));
			let result = if success {
				Ok(confirmed)
			} else {
				Err(client_core::auth::Failure::Network)
			};
			state.apply_edit_result(Id(10), Id(20), *request, result);
			edit_frame(&ctx, &mut view, &mut state, vec![]);
			assert_eq!(state.drafts[&Id(10)], "Unsent draft 👋");
			assert!(view.edit_sent.is_none());
			if success {
				assert!(!view.has_edit());
			} else {
				assert_eq!(view.editing.as_ref().unwrap().2, "Hello :)");
				assert!(state.message_actions.failed_edits.is_empty());
				let retry = edit_frame(
					&ctx,
					&mut view,
					&mut state,
					vec![edit_key(egui::Key::Enter)],
				);
				assert!(
					matches!(retry.as_slice(), [Command::Edit { content, .. }] if content == "Hello 🙂")
				);
			}
		}
	}

	#[test]
	fn emoticon_conversion_failed_noop_edit_and_success_with_newer_input_stay_open() {
		for success in [false, true] {
			let ctx = egui::Context::default();
			let mut state = edit_state();
			let mut original = state.timeline.get(Id(20)).unwrap().clone();
			original.content = "Hello 🙂".into();
			state.timeline.insert(original, true, false).unwrap();
			let mut view = MessagingUi {
				convert_emoticons: true,
				editing: Some((Id(10), Id(20), "Hello :)".into())),
				..Default::default()
			};
			edit_frame(&ctx, &mut view, &mut state, vec![]);
			let commands = edit_frame(
				&ctx,
				&mut view,
				&mut state,
				vec![edit_key(egui::Key::Enter)],
			);
			let [Command::Edit { request, .. }] = commands.as_slice() else {
				panic!()
			};
			if success {
				edit_frame(
					&ctx,
					&mut view,
					&mut state,
					vec![egui::Event::Text(" newer".into())],
				);
			}
			let kept = view.editing.as_ref().unwrap().2.clone();
			let result = if success {
				Ok(state.timeline.get(Id(20)).unwrap().clone())
			} else {
				Err(client_core::auth::Failure::Network)
			};
			state.apply_edit_result(Id(10), Id(20), *request, result);
			edit_frame(&ctx, &mut view, &mut state, vec![]);
			assert_eq!(view.editing.as_ref().unwrap().2, kept);
			assert!(view.edit_sent.is_none());
		}
	}

	fn edit_state() -> State {
		let user = model::User {
			member_avatar: None,
			username: None,
			id: Id(1),
			name: "Alex".into(),
			avatar: None,
			webhook: false,
			kind: Default::default(),
			discriminator: 0,
			primary_guild: None,
			cosmetics: None,
		};
		let mut state = State {
			selected: Some(Id(10)),
			channels: vec![model::Channel {
				id: Id(10),
				guild: None,
				parent_id: None,
				kind: 1,
				name: "Synthetic edit conversation".into(),
				position: 0,
				recipients: vec![],
				last_message: None,
				member_list_id: None,
				tags: None,
				message_count: None,
				icon: None,
			}],
			user: Some(user.clone()),
			demo: true,
			freshness: Freshness::Fresh,
			auth: client_core::auth::AuthState::Authenticated,
			gateway_connected: true,
			..Default::default()
		};
		state
			.timeline
			.insert(
				model::Message {
					reply_preview: None,
					poll: None,
					sticker_items: vec![],
					id: Id(20),
					channel: Id(10),
					author: user,
					content: "Original".into(),
					edited: false,
					edited_at: None,
					revision: 0,
					nonce: None,
					reply_to: None,
					kind: 0,
					reply_deleted: false,
					interaction: None,
					forwarded: false,
					unsupported: false,
					extra_content: Default::default(),
					components: vec![],
					application_id: None,
					ephemeral: false,
					flags: 0,
					embeds: vec![],
					attachments: vec![],
					author_nick: None,
					author_roles: vec![],
					mention_roles: vec![],
					mention_everyone: false,
					suppress_notifications: false,
					mentions: vec![],
					reactions: None,
					embeds_suppressed: false,
				},
				false,
				false,
			)
			.unwrap();
		state.drafts.insert(Id(10), "Unsent draft 👋".into());
		state
	}

	fn edit_frame(
		ctx: &egui::Context,
		view: &mut MessagingUi,
		state: &mut State,
		events: Vec<egui::Event>,
	) -> Vec<Command> {
		let mut commands = vec![];
		let output = ctx.run_ui(
			egui::RawInput {
				events,
				..Default::default()
			},
			|ui| {
				view.composer(ui, state, state.selected.unwrap(), ctx, &mut commands);
			},
		);
		output.drop_without_applying_deltas();
		// This helper simulates key taps; raw-input tests explicitly model held keys.
		ctx.input_mut(|i| i.keys_down.clear());
		commands
	}

	fn edit_key(key: egui::Key) -> egui::Event {
		egui::Event::Key {
			key,
			physical_key: None,
			pressed: true,
			repeat: false,
			modifiers: egui::Modifiers::NONE,
		}
	}

	fn composer_shortcut(key: egui::Key, shift: bool, pressed: bool) -> egui::Event {
		egui::Event::Key {
			key,
			physical_key: None,
			pressed,
			repeat: false,
			modifiers: egui::Modifiers {
				ctrl: true,
				command: true,
				shift,
				..Default::default()
			},
		}
	}

	fn composer_selection(ctx: &egui::Context, widget: egui::Id, start: usize, end: usize) {
		let mut editor = egui::TextEdit::load_state(ctx, widget).unwrap();
		editor
			.cursor
			.set_char_range(Some(egui::text::CCursorRange::two(
				egui::text::CCursor::new(start),
				egui::text::CCursor::new(end),
			)));
		editor.store(ctx, widget);
	}

	#[test]
	fn native_composer_paste_replaces_unicode_selection_and_keeps_typing_order_and_undo() {
		let ctx = egui::Context::default();
		let mut state = edit_state();
		let channel = state.selected.unwrap();
		state.drafts.insert(channel, "aéz".into());
		let mut view = MessagingUi {
			focus_switched_composer: true,
			..Default::default()
		};
		edit_frame(&ctx, &mut view, &mut state, vec![]);
		let widget = ctx.memory(|memory| memory.focused()).unwrap();
		composer_selection(&ctx, widget, 1, 2);
		edit_frame(
			&ctx,
			&mut view,
			&mut state,
			vec![egui::Event::Paste("Ω\r\nB".into())],
		);
		assert_eq!(state.drafts[&channel], "aΩ\nBz");
		assert!(view.attachment_paste_requested.is_none());
		assert!(!view.upload_busy);
		edit_frame(
			&ctx,
			&mut view,
			&mut state,
			vec![egui::Event::Text("!".into())],
		);
		edit_frame(
			&ctx,
			&mut view,
			&mut state,
			vec![egui::Event::Paste("<@2>".into())],
		);
		assert_eq!(state.drafts[&channel], "aΩ\nB!<@2>z");
		edit_frame(
			&ctx,
			&mut view,
			&mut state,
			vec![composer_shortcut(egui::Key::Z, false, true)],
		);
		assert_eq!(
			state.drafts[&channel], "aΩ\nB!z",
			"Undo removes the most recent paste and keeps prior typing"
		);
		edit_frame(
			&ctx,
			&mut view,
			&mut state,
			vec![composer_shortcut(egui::Key::Z, true, true)],
		);
		assert_eq!(state.drafts[&channel], "aΩ\nB!<@2>z");
		let commands = edit_frame(
			&ctx,
			&mut view,
			&mut state,
			vec![edit_key(egui::Key::Enter)],
		);
		assert!(commands.iter().any(
			|command| matches!(command, Command::Send { content, .. } if content == "aΩ\nB!<@2>z")
		));
	}

	#[test]
	fn native_composer_paste_uses_selection_capacity_and_current_frame_navigation() {
		for navigation in [0, 1, 2, 3] {
			let ctx = egui::Context::default();
			let mut state = edit_state();
			let channel = state.selected.unwrap();
			let original = "x".repeat(MAX_CONTENT);
			state.drafts.insert(channel, original.clone());
			let mut view = MessagingUi {
				focus_switched_composer: true,
				..Default::default()
			};
			edit_frame(&ctx, &mut view, &mut state, vec![]);
			let widget = ctx.memory(|memory| memory.focused()).unwrap();
			let mut events = vec![];
			match navigation {
				0 => composer_selection(&ctx, widget, MAX_CONTENT - 1, MAX_CONTENT),
				1 => {
					composer_selection(&ctx, widget, MAX_CONTENT, MAX_CONTENT);
					let mut left = edit_key(egui::Key::ArrowLeft);
					if let egui::Event::Key { modifiers, .. } = &mut left {
						*modifiers = egui::Modifiers::SHIFT;
					}
					events.push(left);
				}
				2 => {
					composer_selection(&ctx, widget, MAX_CONTENT, MAX_CONTENT);
					events.push(edit_key(egui::Key::Backspace));
				}
				_ => {
					composer_selection(&ctx, widget, MAX_CONTENT, MAX_CONTENT);
					events.push(composer_shortcut(egui::Key::Home, false, true));
					let mut right = edit_key(egui::Key::ArrowRight);
					if let egui::Event::Key { modifiers, .. } = &mut right {
						*modifiers = egui::Modifiers::SHIFT;
					}
					events.push(right);
				}
			}
			events.push(egui::Event::Paste("é".into()));
			edit_frame(&ctx, &mut view, &mut state, events);
			let expected = if navigation == 3 {
				format!("é{}", "x".repeat(MAX_CONTENT - 1))
			} else {
				format!("{}é", "x".repeat(MAX_CONTENT - 1))
			};
			assert_eq!(
				state.drafts[&channel], expected,
				"navigation mode {navigation}"
			);
			assert!(
				view.attachment_paste_requested.is_none(),
				"replacement fits after navigation {navigation}"
			);
		}
	}

	#[test]
	fn native_composer_paste_keeps_text_precedence_and_does_not_repeat_on_key_release() {
		let ctx = egui::Context::default();
		let mut state = edit_state();
		let channel = state.selected.unwrap();
		state.drafts.clear();
		let mut view = MessagingUi {
			focus_switched_composer: true,
			..Default::default()
		};
		edit_frame(&ctx, &mut view, &mut state, vec![]);
		let image = std::sync::Arc::new(egui::ColorImage::filled([1, 1], egui::Color32::RED));
		edit_frame(
			&ctx,
			&mut view,
			&mut state,
			vec![
				egui::Event::PasteImage(image.clone()),
				egui::Event::Paste("text".into()),
			],
		);
		assert_eq!(state.drafts[&channel], "text");
		assert!(view.attachment_paste_requested.is_none());
		edit_frame(
			&ctx,
			&mut view,
			&mut state,
			vec![composer_shortcut(egui::Key::V, false, false)],
		);
		assert!(view.attachment_paste_requested.is_none());
		edit_frame(
			&ctx,
			&mut view,
			&mut state,
			vec![egui::Event::PasteImage(image.clone())],
		);
		let request = view.attachment_paste_requested.take().unwrap();
		assert!(std::sync::Arc::ptr_eq(
			request.image.as_ref().unwrap(),
			&image
		));
		assert!(request.text.is_none());
		view.upload_busy = false;
		edit_frame(
			&ctx,
			&mut view,
			&mut state,
			vec![composer_shortcut(egui::Key::V, false, false)],
		);
		assert!(view.attachment_paste_requested.is_none());
		edit_frame(
			&ctx,
			&mut view,
			&mut state,
			vec![composer_shortcut(egui::Key::V, false, false)],
		);
		let request = view
			.attachment_paste_requested
			.take()
			.expect("file-only clipboard key-up still reads native content");
		assert!(request.text.is_none() && request.image.is_none());
	}

	#[test]
	fn menu_and_completed_composer_pastes_allow_the_next_file_only_shortcut() {
		for source in 0..3 {
			let ctx = egui::Context::default();
			let mut state = edit_state();
			let channel = state.selected.unwrap();
			state.drafts.clear();
			let mut view = MessagingUi {
				focus_switched_composer: true,
				..Default::default()
			};
			edit_frame(&ctx, &mut view, &mut state, vec![]);
			let widget = ctx.memory(|memory| memory.focused()).unwrap();
			let events = if source == 2 {
				// A native backend consumes key-down even when it supplies no payload.
				vec![composer_shortcut(egui::Key::V, false, false)]
			} else {
				view.menu_paste = Some((state.generation, channel, widget));
				vec![]
			};
			edit_frame(&ctx, &mut view, &mut state, events);
			let request = view.attachment_paste_requested.take().unwrap();
			view.upload_busy = false;
			if source != 0 {
				view.pasted_text = Some((request.target, "clipboard".into()));
				edit_frame(&ctx, &mut view, &mut state, vec![]);
				assert_eq!(state.drafts[&channel], "clipboard");
				assert!(view.attachment_paste_requested.is_none());
			}
			// Neither menu paste nor asynchronous delivery has a future V release.
			// The next real file-only paste must be admitted on its first key-up.
			edit_frame(
				&ctx,
				&mut view,
				&mut state,
				vec![composer_shortcut(egui::Key::V, false, false)],
			);
			let next = view.attachment_paste_requested.take();
			assert!(
				next.is_some(),
				"Paste source {source} consumed the following file-only shortcut"
			);
			let next = next.unwrap();
			assert_eq!(next.target.widget, widget);
			assert!(next.text.is_none() && next.image.is_none());
		}
	}

	#[test]
	fn option_v_keeps_its_printable_character_and_does_not_start_clipboard_work() {
		let ctx = egui::Context::default();
		ctx.set_os(egui::os::OperatingSystem::Mac);
		let mut state = edit_state();
		let channel = state.selected.unwrap();
		state.drafts.clear();
		let mut view = MessagingUi {
			focus_switched_composer: true,
			..Default::default()
		};
		edit_frame(&ctx, &mut view, &mut state, vec![]);
		let mut option_v = edit_key(egui::Key::V);
		if let egui::Event::Key { modifiers, .. } = &mut option_v {
			*modifiers = egui::Modifiers::ALT;
		}
		edit_frame(
			&ctx,
			&mut view,
			&mut state,
			vec![option_v, egui::Event::Text("√".into())],
		);
		assert_eq!(state.drafts[&channel], "√");
		assert!(view.attachment_paste_requested.is_none());
		assert!(!view.upload_busy);
	}

	#[test]
	fn async_composer_text_paste_requires_the_original_draft_cursor_field_and_account() {
		for change in 0..6 {
			let ctx = egui::Context::default();
			let mut state = edit_state();
			let channel = state.selected.unwrap();
			state.drafts.insert(channel, "abcdef".into());
			let mut view = MessagingUi {
				focus_switched_composer: true,
				..Default::default()
			};
			edit_frame(&ctx, &mut view, &mut state, vec![]);
			let widget = ctx.memory(|memory| memory.focused()).unwrap();
			composer_selection(&ctx, widget, 2, 4);
			view.menu_paste = Some((state.generation, channel, widget));
			edit_frame(&ctx, &mut view, &mut state, vec![]);
			let mut origin = view.attachment_paste_requested.take().unwrap().target;
			view.upload_busy = false;
			let events = match change {
				0 => {
					state.request += 1;
					vec![]
				} // Pagination does not change the paste target.
				1 => {
					state.drafts.insert(channel, "changed".into());
					vec![]
				}
				2 => {
					composer_selection(&ctx, widget, 0, 0);
					vec![]
				}
				3 => {
					origin.widget = widget.with("other-field");
					vec![]
				}
				4 => {
					state.generation += 1;
					vec![]
				}
				_ => vec![egui::Event::Text("!".into())],
			};
			view.pasted_text = Some((origin, "XY".into()));
			edit_frame(&ctx, &mut view, &mut state, events);
			if change == 0 {
				assert_eq!(state.drafts[&channel], "abXYef");
			} else {
				assert_eq!(
					state.status, "Paste cancelled; focus the message and paste again",
					"change {change}"
				);
				assert!(!state.drafts[&channel].contains("XY"));
			}
			assert!(view.pasted_text.is_none());
			assert!(view.attachment_paste_requested.is_none());
		}
	}

	#[test]
	fn native_composer_paste_rejects_oversized_input_without_losing_the_selection() {
		let ctx = egui::Context::default();
		let mut state = edit_state();
		let channel = state.selected.unwrap();
		state.drafts.insert(channel, "keep".into());
		let mut view = MessagingUi {
			focus_switched_composer: true,
			..Default::default()
		};
		edit_frame(&ctx, &mut view, &mut state, vec![]);
		let widget = ctx.memory(|memory| memory.focused()).unwrap();
		composer_selection(&ctx, widget, 0, 4);
		edit_frame(
			&ctx,
			&mut view,
			&mut state,
			vec![egui::Event::Paste("x".repeat(MAX_DRAFT_BYTES + 1))],
		);
		assert_eq!(state.drafts[&channel], "keep");
		assert_eq!(state.status, "Pasted text exceeds the draft limit");
		assert!(
			!egui::TextEdit::load_state(&ctx, widget)
				.unwrap()
				.cursor
				.char_range()
				.unwrap()
				.is_empty()
		);
		assert!(view.attachment_paste_requested.is_none());
	}

	#[test]
	fn native_composer_image_paste_into_an_edit_explains_attachment_unavailability() {
		let ctx = egui::Context::default();
		let mut state = edit_state();
		let channel = state.selected.unwrap();
		let original_draft = state.drafts[&channel].clone();
		let mut view = MessagingUi {
			editing: Some((channel, Id(20), "Original".into())),
			..Default::default()
		};
		edit_frame(&ctx, &mut view, &mut state, vec![]);
		let image = std::sync::Arc::new(egui::ColorImage::filled([1, 1], egui::Color32::RED));
		edit_frame(
			&ctx,
			&mut view,
			&mut state,
			vec![egui::Event::PasteImage(image)],
		);
		assert_eq!(state.status, "Attaching files is unavailable here");
		assert_eq!(view.editing.as_ref().unwrap().2, "Original");
		assert_eq!(state.drafts[&channel], original_draft);
		assert!(view.attachment_paste_requested.is_none());
	}

	#[test]
	fn native_composer_paste_checks_the_session_byte_budget_before_replacing_text() {
		let ctx = egui::Context::default();
		let mut state = edit_state();
		let channel = state.selected.unwrap();
		state.drafts.insert(channel, "ab".into());
		state.drafts.insert(Id(99), "x".repeat(MAX_DRAFT_BYTES - 4));
		let mut view = MessagingUi {
			focus_switched_composer: true,
			..Default::default()
		};
		edit_frame(&ctx, &mut view, &mut state, vec![]);
		let widget = ctx.memory(|memory| memory.focused()).unwrap();
		composer_selection(&ctx, widget, 0, 2);
		edit_frame(
			&ctx,
			&mut view,
			&mut state,
			vec![egui::Event::Paste("éé!".into())],
		);
		assert_eq!(state.drafts[&channel], "ab");
		assert_eq!(state.status, "Pasted text exceeds the draft limit");
		edit_frame(
			&ctx,
			&mut view,
			&mut state,
			vec![egui::Event::Paste("éé".into())],
		);
		assert_eq!(state.drafts[&channel], "éé");
		assert!(state.draft_bytes() <= MAX_DRAFT_BYTES);
		assert!(view.attachment_paste_requested.is_none());
	}

	#[test]
	fn native_composer_paste_keeps_the_existing_large_text_attachment_fallback() {
		let ctx = egui::Context::default();
		let mut state = edit_state();
		let channel = state.selected.unwrap();
		let original = "x".repeat(MAX_CONTENT);
		state.drafts.insert(channel, original.clone());
		let mut view = MessagingUi {
			focus_switched_composer: true,
			..Default::default()
		};
		edit_frame(&ctx, &mut view, &mut state, vec![]);
		edit_frame(
			&ctx,
			&mut view,
			&mut state,
			vec![egui::Event::Paste("overflow".into())],
		);
		let request = view
			.attachment_paste_requested
			.take()
			.expect("text that cannot fit still uses message.txt");
		assert_eq!(request.text.as_deref(), Some("overflow"));
		assert_eq!(request.target.text, original);
		assert_eq!(state.drafts[&channel], original);
		assert!(request.target.text_attachment);
	}

	#[test]
	fn quiet_composer_paste_allows_full_payload_and_marker_only_enter_preserves_draft() {
		let ctx = egui::Context::default();
		let mut view = MessagingUi {
			focus_switched_composer: true,
			..Default::default()
		};
		let mut state = edit_state();
		let channel = state.selected.unwrap();
		state.drafts.insert(channel, "@silent".into());
		for _ in 0..2 {
			edit_frame(&ctx, &mut view, &mut state, vec![]);
		}
		let command = edit_frame(
			&ctx,
			&mut view,
			&mut state,
			vec![edit_key(egui::Key::Enter)],
		);
		assert!(!command.iter().any(|c| matches!(c, Command::Send { .. })));
		assert_eq!(state.drafts[&channel], "@silent");
		assert!(state.pending.is_empty());
		state.drafts.insert(channel, String::new());
		let full = format!("@silent {}", "x".repeat(MAX_CONTENT));
		edit_frame(
			&ctx,
			&mut view,
			&mut state,
			vec![egui::Event::Paste(full.clone())],
		);
		assert!(view.attachment_paste_requested.is_none());
		assert!(!view.upload_busy);
		assert_eq!(state.drafts[&channel], full);
		let command = edit_frame(
			&ctx,
			&mut view,
			&mut state,
			vec![edit_key(egui::Key::Enter)],
		);
		assert!(
			command
				.iter()
				.any(|c| matches!(c, Command::Send { content, .. } if content == &full))
		);
	}

	#[test]
	fn composer_enter_ignores_idle_linux_ime_updates() {
		for editing in [false, true] {
			for shift in [false, true] {
				for ime in [
					egui::ImeEvent::Preedit {
						text: String::new(),
						active_range_chars: None,
					},
					egui::ImeEvent::Preedit {
						text: String::new(),
						active_range_chars: Some(0..0),
					},
					egui::ImeEvent::Commit(String::new()),
				] {
					let ctx = egui::Context::default();
					ctx.set_os(egui::os::OperatingSystem::Nix);
					let mut state = edit_state();
					state.drafts.insert(Id(10), "Message".into());
					let mut view = MessagingUi::default();
					if editing {
						view.editing = Some((Id(10), Id(20), "Changed".into()));
					}
					ctx.run_ui(Default::default(), |ui| {
						view.composer(ui, &mut state, Id(10), &ctx, &mut vec![]);
						ctx.memory_mut(|m| {
							m.request_focus(ui.make_persistent_id(if editing {
								"message-edit"
							} else {
								"message-input"
							}))
						});
					})
					.drop_without_applying_deltas();
					let mut enter = edit_key(egui::Key::Enter);
					if let egui::Event::Key { modifiers, .. } = &mut enter {
						modifiers.shift = shift;
					}
					let commands = edit_frame(
						&ctx,
						&mut view,
						&mut state,
						vec![egui::Event::Ime(ime), enter],
					);
					if shift {
						assert!(commands.is_empty());
						let text = if editing {
							&view.editing.as_ref().unwrap().2
						} else {
							&state.drafts[&Id(10)]
						};
						assert!(text.contains('\n'));
					} else if editing {
						assert!(
							matches!(commands.as_slice(), [Command::Edit { content, .. }] if content == "Changed")
						);
					} else {
						assert!(
							matches!(commands.as_slice(), [Command::Send { content, .. }] if content == "Message")
						);
					}
				}
			}
		}
	}

	#[test]
	fn composer_enter_dismissing_composition_does_not_send() {
		for finish in [
			egui::ImeEvent::Preedit {
				text: String::new(),
				active_range_chars: None,
			},
			egui::ImeEvent::Commit(String::new()),
			egui::ImeEvent::Commit("composed".into()),
		] {
			let ctx = egui::Context::default();
			ctx.set_os(egui::os::OperatingSystem::Nix);
			let mut state = edit_state();
			let mut view = MessagingUi::default();
			ctx.run_ui(Default::default(), |ui| {
				view.composer(ui, &mut state, Id(10), &ctx, &mut vec![]);
				ctx.memory_mut(|m| m.request_focus(ui.make_persistent_id("message-input")));
			})
			.drop_without_applying_deltas();
			assert!(
				edit_frame(
					&ctx,
					&mut view,
					&mut state,
					vec![egui::Event::Ime(egui::ImeEvent::Preedit {
						text: "composition".into(),
						active_range_chars: None,
					})]
				)
				.is_empty()
			);
			assert!(view.ime_active);
			assert!(
				edit_frame(
					&ctx,
					&mut view,
					&mut state,
					vec![egui::Event::Ime(finish), edit_key(egui::Key::Enter)]
				)
				.is_empty()
			);
			assert!(!view.ime_active);
			assert!(matches!(
				edit_frame(
					&ctx,
					&mut view,
					&mut state,
					vec![
						egui::Event::Ime(egui::ImeEvent::Preedit {
							text: String::new(),
							active_range_chars: None,
						}),
						edit_key(egui::Key::Enter)
					]
				)
				.as_slice(),
				[Command::Send { .. }]
			));
		}
	}

	#[test]
	fn joined_invite_cards_open_servers_without_joining_and_preserve_drafts() {
		fn button_rect(shape: &egui::Shape, label: &str) -> Option<egui::Rect> {
			match shape {
				egui::Shape::Text(text) if text.galley.job.text == label => {
					Some(text.galley.rect.translate(text.pos.to_vec2()))
				}
				egui::Shape::Vec(shapes) => {
					shapes.iter().find_map(|shape| button_rect(shape, label))
				}
				_ => None,
			}
		}
		let ctx = egui::Context::default();
		design::apply(&ctx);
		let mut state = edit_state();
		state.demo = false;
		state.guilds.push(model::Guild {
			default_message_notifications: None,
			stickers: None,
			id: Id(100),
			name: "Synthetic invited server".into(),
			icon: None,
			emojis: None,
		});
		let mut channel = state.channels[0].clone();
		channel.id = Id(12);
		channel.guild = Some(Id(100));
		channel.kind = 0;
		state.channels.push(channel);
		state.last_viewed_channels.push((Id(100), Id(12)));
		state
			.permissions
			.replace(test_support::permission_snapshot(&state))
			.unwrap();
		let mut message = state.timeline.get(Id(20)).unwrap().clone();
		message.content = "https://discord.gg/synthetic".into();
		state.timeline.insert(message, true, false).unwrap();
		state.invites.insert(
			"synthetic".into(),
			(
				std::time::Instant::now(),
				Some(Ok(model::InvitePreview {
					guild: Id(100),
					embed: model::Embed {
						title: Some("Synthetic invited server".into()),
						..Default::default()
					},
				})),
			),
		);
		let mut view = MessagingUi::default();
		view.reading_preferences.show_members = false;
		let drafts = state.drafts.clone();
		let frame = |view: &mut MessagingUi, state: &mut State, events| {
			let mut commands = vec![];
			let output = ctx.run_ui(
				egui::RawInput {
					screen_rect: Some(egui::Rect::from_min_size(
						egui::Pos2::ZERO,
						egui::vec2(760.0, 700.0),
					)),
					events,
					..Default::default()
				},
				|ui| commands = view.show(ui, state),
			);
			let rect = output
				.shapes
				.iter()
				.find_map(|shape| button_rect(&shape.shape, "Go To Server"));
			output.drop_without_applying_deltas();
			(rect, commands)
		};
		for _ in 0..3 {
			frame(&mut view, &mut state, vec![]);
		}
		let point = frame(&mut view, &mut state, vec![])
			.0
			.expect("Go To Server")
			.center();
		let mut commands = vec![];
		for pressed in [true, false] {
			commands.extend(
				frame(
					&mut view,
					&mut state,
					vec![
						egui::Event::PointerMoved(point),
						egui::Event::PointerButton {
							pos: point,
							button: egui::PointerButton::Primary,
							pressed,
							modifiers: egui::Modifiers::NONE,
						},
					],
				)
				.1,
			);
		}
		commands.extend(frame(&mut view, &mut state, vec![]).1);
		assert_eq!((view.guild, state.selected), (Some(Id(100)), Some(Id(12))));
		assert_eq!(state.drafts, drafts);
		assert!(
			!commands
				.iter()
				.any(|command| matches!(command, Command::JoinInvite { .. }))
		);
		assert_eq!(
			commands
				.iter()
				.filter(|command| matches!(
					command,
					Command::History {
						channel: Id(12),
						..
					}
				))
				.count(),
			1
		);
	}

	#[test]
	fn discord_chat_links_navigate_from_message_text_without_opening_a_browser() {
		fn link_rect(shape: &egui::Shape, label: &str) -> Option<egui::Rect> {
			match shape {
				egui::Shape::Text(text) if text.galley.job.text.trim() == label => {
					Some(text.galley.rect.translate(text.pos.to_vec2()))
				}
				egui::Shape::Vec(shapes) => shapes.iter().find_map(|shape| link_rect(shape, label)),
				_ => None,
			}
		}
		for (source, label, target_message) in [
			(
				"https://discord.com/channels/100/11",
				"https://discord.com/channels/100/11",
				None,
			),
			(
				"https://discord.com/channels/100/11/25",
				"Linked server",
				Some(Id(25)),
			),
			(
				"<HTTPS://DISCORD.COM:443/channels/100/11/25?jump=1#message>",
				"Linked server",
				Some(Id(25)),
			),
			(
				"[Other chat](https://discord.com/channels/100/11/25)",
				"Other chat",
				Some(Id(25)),
			),
		] {
			let ctx = egui::Context::default();
			let mut view = MessagingUi::default();
			let mut state = edit_state();
			let mut target = state.channels[0].clone();
			target.id = Id(11);
			target.guild = Some(Id(100));
			target.kind = 0;
			state.channels.push(target);
			state.guilds.push(model::Guild {
				default_message_notifications: None,
				stickers: None,
				id: Id(100),
				name: "Linked server".into(),
				icon: None,
				emojis: None,
			});
			state
				.permissions
				.replace(test_support::permission_snapshot(&state))
				.unwrap();
			let mut message = state.timeline.get(Id(20)).unwrap().clone();
			message.content = source.into();
			state.timeline.insert(message, true, false).unwrap();
			let drafts = state.drafts.clone();
			let frame = |view: &mut MessagingUi, state: &mut State, events| {
				let mut commands = vec![];
				let output = ctx.run_ui(
					egui::RawInput {
						screen_rect: Some(egui::Rect::from_min_size(
							egui::Pos2::ZERO,
							egui::vec2(1000.0, 700.0),
						)),
						events,
						..Default::default()
					},
					|ui| commands = view.show(ui, state),
				);
				assert!(
					!output
						.platform_output
						.commands
						.iter()
						.any(|command| matches!(command, egui::OutputCommand::OpenUrl(_)))
				);
				let rect = output
					.shapes
					.iter()
					.find_map(|shape| link_rect(&shape.shape, label));
				output.drop_without_applying_deltas();
				(rect, commands)
			};
			for _ in 0..3 {
				frame(&mut view, &mut state, vec![]);
			}
			// The chip's atomic URL selection slot identifies its exact hit geometry.
			// The resolved name also appears in the channel list/header, so it cannot
			// reliably identify the message-body link here.
			let point = frame(&mut view, &mut state, vec![])
				.0
				.expect("rendered chat link")
				.center();
			let mut commands = vec![];
			for pressed in [true, false] {
				commands.extend(
					frame(
						&mut view,
						&mut state,
						vec![
							egui::Event::PointerMoved(point),
							egui::Event::PointerButton {
								pos: point,
								button: egui::PointerButton::Primary,
								pressed,
								modifiers: egui::Modifiers::NONE,
							},
						],
					)
					.1,
				);
			}
			commands.extend(frame(&mut view, &mut state, vec![]).1);
			assert_eq!(state.selected, Some(Id(11)), "source: {source}");
			assert_eq!(view.guild, Some(Id(100)));
			assert!(
				state.search_target.is_none(),
				"Remote targets are published only after a validated page succeeds"
			);
			assert_eq!(state.drafts, drafts);
			assert_eq!(
				commands
					.iter()
					.filter(|command| match command {
						Command::HistoryAround {
							channel: Id(11),
							target,
							..
						} => Some(*target) == target_message,
						Command::History {
							channel: Id(11),
							before: None,
							..
						} => target_message.is_none(),
						_ => false,
					})
					.count(),
				1
			);
			assert!(!commands.iter().any(|command| matches!(
				command,
				Command::Send { .. } | Command::Edit { .. } | Command::Delete { .. }
			)));
		}
	}

	#[test]
	fn malformed_discord_origins_dispatch_externally_without_native_routes_or_metadata() {
		fn text_rect(shape: &egui::Shape, label: &str) -> Option<egui::Rect> {
			match shape {
				egui::Shape::Text(text) if text.galley.text().trim() == label => {
					Some(text.galley.rect.translate(text.pos.to_vec2()))
				}
				egui::Shape::Vec(shapes) => shapes.iter().find_map(|shape| text_rect(shape, label)),
				_ => None,
			}
		}
		for (raw, external, confirms) in [
			(
				"https://discord.com/not/../channels/100/11/25?jump=1#message",
				"https://discord.com/channels/100/11/25?jump=1#message",
				false,
			),
			(
				"https://discord.com/not/%2e%2e/channels/100/11/25?jump=1#message",
				"https://discord.com/channels/100/11/25?jump=1#message",
				false,
			),
			(
				"https://discord%2Ecom/channels/100/11/25",
				"https://discord.com/channels/100/11/25",
				false,
			),
			(
				"HTTPS://DISCORD.COM.EVIL.EXAMPLE:443/not/../channels/100/11/25?jump=1#message",
				"https://discord.com.evil.example/channels/100/11/25?jump=1#message",
				true,
			),
		] {
			assert!(markdown::discord_chat_link(raw).is_none());
			assert_eq!(markdown::external_url(raw).as_deref(), Some(external));
			for source in [raw.to_owned(), format!("<{raw}>")] {
				let ctx = egui::Context::default();
				let mut view = MessagingUi::default();
				view.reading_preferences.confirm_external_links = true;
				let mut state = edit_state();
				let mut target = state.channels[0].clone();
				target.id = Id(11);
				target.guild = Some(Id(100));
				target.kind = 0;
				target.name = "Malformed-origin cached target".into();
				state.channels.push(target);
				state.guilds.push(model::Guild {
					id: Id(100),
					name: "Malformed-origin cached server".into(),
					icon: None,
					emojis: None,
					stickers: None,
					default_message_notifications: None,
				});
				state
					.permissions
					.replace(test_support::permission_snapshot(&state))
					.unwrap();
				let mut message = state.timeline.get(Id(20)).unwrap().clone();
				message.content = source;
				state.timeline.insert(message, true, false).unwrap();
				let drafts = state.drafts.clone();
				let frame =
					|view: &mut MessagingUi, state: &mut State, events| {
						let mut commands = vec![];
						let output = ctx.run_ui(
							egui::RawInput {
								screen_rect: Some(egui::Rect::from_min_size(
									egui::Pos2::ZERO,
									egui::vec2(1000.0, 700.0),
								)),
								events,
								..Default::default()
							},
							|ui| commands = view.show(ui, state),
						);
						assert_eq!(state.selected, Some(Id(10)), "{raw}");
						assert!(
							state.search_target.is_none() && view.guild.is_none(),
							"{raw}"
						);
						assert_eq!(state.drafts, drafts);
						assert!(!commands.iter().any(|command| matches!(
							command,
							Command::History {
								channel: Id(11),
								..
							} | Command::Send { .. } | Command::Edit { .. }
								| Command::Delete { .. } | Command::Voice(_)
						)));
						for label in [
							"Malformed-origin cached target",
							"Malformed-origin cached server",
							"Unknown channel",
						] {
							assert!(
								output
									.shapes
									.iter()
									.all(|shape| text_rect(&shape.shape, label).is_none()),
								"No native destination chip for {raw}"
							);
						}
						assert!(output.platform_output.events.iter().all(|event| {
							let info = event.widget_info();
							info.role != egui::Role::Link
								|| info.label.as_deref().is_none_or(|label| {
									!label.contains("Jump to message")
										&& !label.contains("Malformed-origin cached")
								})
						}));
						let find = |label| {
							output
								.shapes
								.iter()
								.find_map(|shape| text_rect(&shape.shape, label))
						};
						let link = find(raw);
						let confirm = find("Open in Browser");
						let destination = find(external).is_some();
						let opened: Vec<_> = output
							.platform_output
							.commands
							.iter()
							.filter_map(|command| match command {
								egui::OutputCommand::OpenUrl(url) => Some(url.url.clone()),
								_ => None,
							})
							.collect();
						output.drop_without_applying_deltas();
						(link, confirm, destination, opened)
					};
				for _ in 0..3 {
					assert!(frame(&mut view, &mut state, vec![]).3.is_empty());
				}
				let point = frame(&mut view, &mut state, vec![])
					.0
					.expect("raw link label")
					.center();
				let click = |point, pressed| {
					vec![
						egui::Event::PointerMoved(point),
						egui::Event::PointerButton {
							pos: point,
							button: egui::PointerButton::Primary,
							pressed,
							modifiers: egui::Modifiers::NONE,
						},
					]
				};
				let mut opened = vec![];
				for pressed in [true, false] {
					opened.extend(frame(&mut view, &mut state, click(point, pressed)).3);
				}
				opened.extend(frame(&mut view, &mut state, vec![]).3);
				if confirms {
					assert!(opened.is_empty(), "Spoof hosts require confirmation");
					assert!(view.timeline.opening.is_some());
					let mut confirm = None;
					for _ in 0..3 {
						let (_, button, displayed, emitted) = frame(&mut view, &mut state, vec![]);
						assert!(emitted.is_empty());
						assert!(displayed, "Confirmation must show the actual destination");
						confirm = button;
					}
					let point = confirm.expect("external confirmation action").center();
					for pressed in [true, false] {
						opened.extend(frame(&mut view, &mut state, click(point, pressed)).3);
					}
				}
				// Genuine Discord origins retain the existing external-open exemption, but
				// neither raw path attacks nor encoded authorities become native navigation.
				assert_eq!(opened, [external.to_owned()], "{raw}");
				assert!(view.timeline.opening.is_none());
				assert!(frame(&mut view, &mut state, vec![]).3.is_empty());
			}
		}
	}

	#[test]
	fn expanded_provider_original_reaches_the_shared_link_confirmation() {
		let ctx = egui::Context::default();
		crate::design::apply(&ctx);
		let mut state = test_support::demo_state();
		let channel = state.selected.unwrap();
		let source = "https://youtu.be/synthet1c01";
		let mut message = test_support::message(600, channel);
		message.attachments.clear();
		message.embeds = vec![model::Embed {
			kind: "video".into(),
			url: Some(source.into()),
			..Default::default()
		}];
		state.timeline.clear();
		state.timeline.seed_cache(vec![message.clone()]).unwrap();
		let mut view = MessagingUi::default();
		view.reading_preferences.confirm_external_links = true;
		let frame = |view: &mut MessagingUi, state: &mut State, events| {
			ctx.run_ui(
				egui::RawInput {
					screen_rect: Some(egui::Rect::from_min_size(
						egui::Pos2::ZERO,
						egui::vec2(1000.0, 800.0),
					)),
					events,
					..Default::default()
				},
				|ui| {
					let _ = view.show(ui, state);
				},
			)
		};
		fn point(output: &egui::FullOutput, label: &str) -> egui::Pos2 {
			output
				.shapes
				.iter()
				.find_map(|shape| match &shape.shape {
					egui::Shape::Text(text) if text.galley.job.text == label => {
						Some(text.pos + text.galley.rect.center().to_vec2())
					}
					_ => None,
				})
				.unwrap_or_else(|| panic!("missing {label}"))
		}
		let click = |pos, pressed| {
			vec![
				egui::Event::PointerMoved(pos),
				egui::Event::PointerButton {
					pos,
					button: egui::PointerButton::Primary,
					pressed,
					modifiers: egui::Modifiers::NONE,
				},
			]
		};
		for _ in 0..3 {
			frame(&mut view, &mut state, vec![]).drop_without_applying_deltas();
		}
		view.timeline.video.open_provider(
			&message,
			model::YouTubeVideo::new("synthet1c01", 0).unwrap(),
			source.into(),
			state.generation,
		);
		view.timeline.video.provider.as_mut().unwrap().expanded = true;
		for _ in 0..3 {
			frame(&mut view, &mut state, vec![]).drop_without_applying_deltas();
		}
		let output = frame(&mut view, &mut state, vec![]);
		let original = point(
			&output,
			&crate::i18n::translate("video-provider-open-original"),
		);
		output.drop_without_applying_deltas();
		for pressed in [true, false] {
			let output = frame(&mut view, &mut state, click(original, pressed));
			assert!(
				!output
					.platform_output
					.commands
					.iter()
					.any(|command| matches!(command, egui::OutputCommand::OpenUrl(_)))
			);
			output.drop_without_applying_deltas();
		}
		assert_eq!(view.timeline.opening.as_deref(), Some(source));
		assert!(!view.timeline.video.provider.as_ref().unwrap().expanded);
		for _ in 0..3 {
			frame(&mut view, &mut state, vec![]).drop_without_applying_deltas();
		}
		let output = frame(&mut view, &mut state, vec![]);
		let confirm = point(
			&output,
			&crate::i18n::translate("markdown-confirm-external-link-open-in-browser"),
		);
		output.drop_without_applying_deltas();
		let mut opened = Vec::new();
		for pressed in [true, false] {
			let output = frame(&mut view, &mut state, click(confirm, pressed));
			opened.extend(output.platform_output.commands.iter().filter_map(
				|command| match command {
					egui::OutputCommand::OpenUrl(url) => Some(url.url.clone()),
					_ => None,
				},
			));
			output.drop_without_applying_deltas();
		}
		assert_eq!(opened, [source]);
		assert!(view.timeline.opening.is_none());
	}

	#[test]
	fn link_confirmation_escape_preserves_background_search_and_works_without_selection() {
		let ctx = egui::Context::default();
		let mut state = edit_state();
		let mut view = MessagingUi::default();
		view.reading_preferences.confirm_external_links = true;
		let frame = |view: &mut MessagingUi, state: &mut State, events| {
			let output = ctx.run_ui(
				egui::RawInput {
					screen_rect: Some(egui::Rect::from_min_size(
						egui::Pos2::ZERO,
						egui::vec2(1000.0, 700.0),
					)),
					events,
					..Default::default()
				},
				|ui| {
					view.show(ui, state);
				},
			);
			let emitted = output.platform_output.commands.len();
			output.drop_without_applying_deltas();
			assert_eq!(
				emitted, 0,
				"Showing or canceling a link never opens a browser"
			);
		};
		frame(&mut view, &mut state, vec![]);
		view.search.open = true;
		view.timeline.opening = Some("https://example.com/message".into());
		for _ in 0..3 {
			frame(&mut view, &mut state, vec![]);
		}
		frame(&mut view, &mut state, vec![edit_key(egui::Key::Escape)]);
		assert!(view.timeline.opening.is_none());
		assert!(
			view.search.open,
			"Escape belongs to the foreground confirmation"
		);
		state.selected = None;
		view.timeline.opening = Some("https://example.com/conversation".into());
		for _ in 0..3 {
			frame(&mut view, &mut state, vec![]);
		}
		assert!(view.timeline.opening.is_some());
		frame(&mut view, &mut state, vec![edit_key(egui::Key::Escape)]);
		assert!(view.timeline.opening.is_none());
	}

	#[test]
	fn conversation_shortcut_preserves_edits_and_drafts_and_never_sends() {
		fn frame(
			ctx: &egui::Context,
			view: &mut MessagingUi,
			state: &mut State,
			events: Vec<egui::Event>,
		) -> Vec<Command> {
			let mut commands = Vec::new();
			let output = ctx.run_ui(
				egui::RawInput {
					screen_rect: Some(egui::Rect::from_min_size(
						egui::Pos2::ZERO,
						egui::vec2(1200.0, 800.0),
					)),
					focused: true,
					events,
					..Default::default()
				},
				|ui| commands.extend(view.show(ui, state)),
			);
			output.drop_without_applying_deltas();
			assert!(!commands.iter().any(|command| matches!(
				command,
				Command::Send { .. } | Command::Edit { .. } | Command::Voice(_)
			)));
			commands
		}
		let shortcut = || egui::Event::Key {
			key: egui::Key::K,
			physical_key: None,
			pressed: true,
			repeat: false,
			modifiers: egui::Modifiers::COMMAND,
		};
		let ctx = egui::Context::default();
		let mut state = edit_state();
		let mut other = state.channels[0].clone();
		other.id = Id(11);
		other.name = "Other conversation".into();
		state.channels.push(other);
		state.drafts.insert(Id(11), "Another unsent draft".into());
		let drafts = state.drafts.clone();
		let mut view = MessagingUi {
			editing: Some((Id(10), Id(20), "Keep this unfinished edit".into())),
			..Default::default()
		};
		frame(&ctx, &mut view, &mut state, vec![]);
		view.ime_active = true;
		frame(&ctx, &mut view, &mut state, vec![shortcut()]);
		assert!(
			!view.switcher.is_open(),
			"Do not interrupt an active composition"
		);
		view.ime_active = false;
		frame(&ctx, &mut view, &mut state, vec![shortcut()]);
		assert!(view.switcher.is_open());
		frame(&ctx, &mut view, &mut state, vec![]);
		frame(
			&ctx,
			&mut view,
			&mut state,
			vec![edit_key(egui::Key::Escape)],
		);
		assert!(!view.switcher.is_open());
		assert_eq!(state.selected, Some(Id(10)));
		assert_eq!(
			view.editing.as_ref().unwrap().2,
			"Keep this unfinished edit"
		);
		frame(&ctx, &mut view, &mut state, vec![shortcut()]);
		frame(&ctx, &mut view, &mut state, vec![]);
		frame(
			&ctx,
			&mut view,
			&mut state,
			vec![edit_key(egui::Key::ArrowDown)],
		);
		let commands = frame(
			&ctx,
			&mut view,
			&mut state,
			vec![edit_key(egui::Key::Enter)],
		);
		assert_eq!(state.selected, Some(Id(11)));
		assert!(commands.iter().any(|command| matches!(
			command,
			Command::History {
				channel: Id(11),
				..
			}
		)));
		assert!(!view.switcher.is_open());
		assert_eq!(state.drafts, drafts);
		assert_eq!(
			view.editing.as_ref().unwrap().2,
			"Keep this unfinished edit"
		);
		for _ in 0..3 {
			frame(&ctx, &mut view, &mut state, vec![]);
		}
		assert!(!view.focus_switched_composer);
		frame(
			&ctx,
			&mut view,
			&mut state,
			vec![egui::Event::Text(" typed".into())],
		);
		assert_eq!(state.drafts[&Id(11)], "Another unsent draft typed");
		assert_eq!(state.drafts[&Id(10)], drafts[&Id(10)]);
		assert_eq!(
			view.editing.as_ref().unwrap().2,
			"Keep this unfinished edit"
		);
	}

	#[test]
	fn switcher_pointer_close_during_ime_cannot_commit_into_the_composer() {
		let ctx = egui::Context::default();
		let mut state = edit_state();
		let drafts = state.drafts.clone();
		let mut view = MessagingUi {
			editing: Some((Id(10), Id(20), "Keep this edit".into())),
			..Default::default()
		};
		let frame = |view: &mut MessagingUi, state: &mut State, events| {
			let mut commands = Vec::new();
			let output = ctx.run_ui(
				egui::RawInput {
					screen_rect: Some(egui::Rect::from_min_size(
						egui::Pos2::ZERO,
						egui::vec2(1200.0, 800.0),
					)),
					focused: true,
					events,
					..Default::default()
				},
				|ui| commands.extend(view.show(ui, state)),
			);
			assert!(
				!commands
					.iter()
					.any(|command| matches!(command, Command::Send { .. } | Command::Edit { .. }))
			);
			output
		};
		frame(&mut view, &mut state, vec![]).drop_without_applying_deltas();
		view.switcher.open(&ctx);
		frame(&mut view, &mut state, vec![]).drop_without_applying_deltas();
		let output = frame(&mut view, &mut state, vec![]);
		let close = output
			.shapes
			.iter()
			.find_map(|shape| match &shape.shape {
				egui::Shape::Text(text) if text.galley.job.text == "Close" => {
					Some(text.pos + text.galley.size() / 2.0)
				}
				_ => None,
			})
			.expect("Rendered picker Close control");
		output.drop_without_applying_deltas();
		frame(
			&mut view,
			&mut state,
			vec![egui::Event::Ime(egui::ImeEvent::Preedit {
				text: "Query composition".into(),
				active_range_chars: None,
			})],
		)
		.drop_without_applying_deltas();
		for pressed in [true, false] {
			frame(
				&mut view,
				&mut state,
				vec![
					egui::Event::PointerMoved(close),
					egui::Event::PointerButton {
						pos: close,
						button: egui::PointerButton::Primary,
						pressed,
						modifiers: egui::Modifiers::NONE,
					},
				],
			)
			.drop_without_applying_deltas();
		}
		assert!(
			view.switcher.is_open(),
			"Pointer close must wait for the query composition"
		);
		frame(
			&mut view,
			&mut state,
			vec![egui::Event::Ime(egui::ImeEvent::Commit(
				"Query composition".into(),
			))],
		)
		.drop_without_applying_deltas();
		frame(&mut view, &mut state, vec![edit_key(egui::Key::Escape)])
			.drop_without_applying_deltas();
		frame(&mut view, &mut state, vec![]).drop_without_applying_deltas();
		assert!(!view.switcher.is_open());
		assert!(
			!view.ime_active,
			"Query IME state must not leak into the composer"
		);
		assert_eq!(state.selected, Some(Id(10)));
		assert_eq!(state.drafts, drafts);
		assert_eq!(view.editing.as_ref().unwrap().2, "Keep this edit");
	}

	#[test]
	fn inline_edit_uses_mentions_ime_and_preserves_draft_with_optimistic_updates() {
		// Restoring failed edits needs the full shell; keep editor IDs stable across frames.
		let edit_frame =
			|ctx: &egui::Context, view: &mut MessagingUi, state: &mut State, events| {
				let mut commands = Vec::new();
				let output = ctx.run_ui(
					egui::RawInput {
						screen_rect: Some(egui::Rect::from_min_size(
							egui::Pos2::ZERO,
							egui::vec2(1000.0, 700.0),
						)),
						events,
						..Default::default()
					},
					|ui| commands.extend(view.show(ui, state)),
				);
				output.drop_without_applying_deltas();
				ctx.input_mut(|input| input.keys_down.clear());
				commands
			};
		let ctx = egui::Context::default();
		let mut state = edit_state();
		let mut view = MessagingUi {
			editing: Some((Id(10), Id(20), "Original".into())),
			..Default::default()
		};
		view.reading_preferences.show_members = false;
		assert!(edit_frame(&ctx, &mut view, &mut state, vec![]).is_empty());
		assert!(
			edit_frame(
				&ctx,
				&mut view,
				&mut state,
				vec![egui::Event::Text(" @Al".into())]
			)
			.is_empty()
		);
		assert!(
			edit_frame(
				&ctx,
				&mut view,
				&mut state,
				vec![edit_key(egui::Key::Enter)]
			)
			.is_empty()
		);
		assert_eq!(view.editing.as_ref().unwrap().2, "Original <@1> ");
		assert!(
			edit_frame(
				&ctx,
				&mut view,
				&mut state,
				vec![
					egui::Event::Ime(egui::ImeEvent::Commit("語".into())),
					edit_key(egui::Key::Enter)
				]
			)
			.is_empty()
		);
		let commands = edit_frame(
			&ctx,
			&mut view,
			&mut state,
			vec![edit_key(egui::Key::Enter)],
		);
		assert!(
			matches!(commands.as_slice(), [Command::Edit { channel: Id(10), message: Id(20), content, .. }] if content.trim_end() == "Original <@1> 語")
		);
		assert!(view.has_edit(), "optimistic edits await confirmation");
		assert_eq!(
			state.timeline.get(Id(20)).unwrap().content,
			"Original <@1> 語\n"
		);
		let [Command::Edit { request, .. }] = commands.as_slice() else {
			panic!("Saving the edit emits one edit command");
		};
		state.apply_edit_result(
			Id(10),
			Id(20),
			*request,
			Err(client_core::auth::Failure::Network),
		);
		assert_eq!(state.timeline.get(Id(20)).unwrap().content, "Original");
		assert_eq!(
			state.message_actions.failed_edits[0].2,
			"Original <@1> 語\n"
		);
		assert_eq!(state.drafts[&Id(10)], "Unsent draft 👋");
		assert!(
			view.draft_changes.is_empty(),
			"edits must not overwrite unsent drafts"
		);
		assert!(edit_frame(&ctx, &mut view, &mut state, vec![]).is_empty());
		assert_eq!(view.editing.as_ref().unwrap().2, "Original <@1> 語\n");
		assert!(state.message_actions.failed_edits.is_empty());
		let commands = edit_frame(
			&ctx,
			&mut view,
			&mut state,
			vec![edit_key(egui::Key::Enter)],
		);
		let [Command::Edit { request, .. }] = commands.as_slice() else {
			panic!()
		};
		assert!(view.has_edit());
		let confirmed = state.timeline.get(Id(20)).unwrap().clone();
		view.editing = Some((Id(10), Id(20), "Newer input".into()));
		state.apply_edit_result(Id(10), Id(20), *request, Ok(confirmed));
		edit_frame(&ctx, &mut view, &mut state, vec![]);
		assert_eq!(view.editing.as_ref().unwrap().2, "Newer input");
		assert_eq!(state.drafts[&Id(10)], "Unsent draft 👋");
	}

	#[test]
	fn inline_edit_survives_navigation_blocks_invalid_saves_and_cancels_without_sending() {
		let ctx = egui::Context::default();
		let mut state = edit_state();
		let mut view = MessagingUi {
			editing: Some((Id(10), Id(20), "Replacement".into())),
			..Default::default()
		};
		edit_frame(&ctx, &mut view, &mut state, vec![]);
		state.selected = Some(Id(11));
		edit_frame(&ctx, &mut view, &mut state, vec![]);
		assert_eq!(view.editing.as_ref().unwrap().2, "Replacement");
		assert!(!state.drafts.contains_key(&Id(11)));
		state.selected = Some(Id(10));
		edit_frame(&ctx, &mut view, &mut state, vec![]);
		state.freshness = Freshness::Stale;
		assert!(
			edit_frame(
				&ctx,
				&mut view,
				&mut state,
				vec![edit_key(egui::Key::Enter)]
			)
			.is_empty()
		);
		assert!(view.has_edit());
		state.freshness = Freshness::Fresh;
		let channels = std::mem::take(&mut state.channels);
		assert!(
			edit_frame(
				&ctx,
				&mut view,
				&mut state,
				vec![edit_key(egui::Key::Enter)]
			)
			.is_empty()
		);
		assert!(
			view.has_edit(),
			"Losing channel access keeps the unsaved edit"
		);
		state.channels = channels;
		state.user.as_mut().unwrap().id = Id(99);
		assert!(
			edit_frame(
				&ctx,
				&mut view,
				&mut state,
				vec![edit_key(egui::Key::Enter)]
			)
			.is_empty()
		);
		assert!(view.has_edit());
		assert!(
			edit_frame(
				&ctx,
				&mut view,
				&mut state,
				vec![edit_key(egui::Key::Escape)]
			)
			.is_empty()
		);
		assert!(!view.has_edit());
		assert_eq!(state.drafts[&Id(10)], "Unsent draft 👋");
		assert!(view.draft_changes.is_empty());
	}

	#[test]
	fn unread_pages_and_return_to_present_preserve_drafts_without_automatic_ack() {
		use crate::test_ui::logical_text as collect;
		for (marker, width, dark) in [(None, 760.0, false), (Some(Id(10)), 1100.0, true)] {
			let ctx = egui::Context::default();
			ctx.set_visuals(if dark {
				egui::Visuals::dark()
			} else {
				egui::Visuals::light()
			});
			let mut view = MessagingUi::default();
			let mut state = edit_state();
			state.channels[0].last_message = Some(Id(20));
			state
				.apply_read_state(client_core::read_state::Event::Snapshot {
					entries: Some(vec![(Id(10), marker, 0)]),
					version: Some(1),
					partial: false,
				})
				.unwrap();
			let mut message = state.timeline.get(Id(20)).unwrap().clone();
			message.content = "Synthetic tall unread row\n\n".repeat(80);
			state.timeline.clear();
			state
				.timeline
				.insert(message.clone(), false, false)
				.unwrap();
			state.revision += 1;
			let drafts = state.drafts.clone();
			let frame = |view: &mut MessagingUi, state: &mut State, events| {
				let mut commands = vec![];
				let output = ctx.run_ui(
					egui::RawInput {
						focused: true,
						events,
						screen_rect: Some(egui::Rect::from_min_size(
							egui::Pos2::ZERO,
							egui::vec2(width, 720.0),
						)),
						..Default::default()
					},
					|ui| commands = view.show(ui, state),
				);
				// egui reports settled text selection for the native PRIMARY clipboard.
				// Explicit clipboard writes and browser actions must remain absent.
				assert!(
					output
						.platform_output
						.commands
						.iter()
						.all(|command| matches!(
							command,
							egui::OutputCommand::TextSelectionSettled(_)
						)),
					"Unexpected platform actions: {:?}",
					output.platform_output.commands
				);
				assert!(
					!commands.iter().any(|command| matches!(
						command,
						Command::Send { .. }
							| Command::Edit { .. }
							| Command::MarkRead { .. }
							| Command::MarkGuildRead { .. }
					)),
					"Browsing unread pages must not send or acknowledge"
				);
				let mut labels = vec![];
				for shape in &output.shapes {
					collect(&shape.shape, &mut labels);
				}
				output.drop_without_applying_deltas();
				(labels, commands)
			};
			let click = |view: &mut MessagingUi, state: &mut State, label: &str| {
				let (labels, _) = frame(view, state, vec![]);
				// The return to the live edge is an icon-only control with no painted text.
				let pos = if label == "Jump to present" {
					view.timeline
						.present_control
						.expect("navigation control")
						.center()
				} else {
					labels
						.iter()
						.find(|(text, _)| text == label)
						.expect("navigation control")
						.1
						.center()
				};
				let mut commands = vec![];
				for pressed in [true, false] {
					commands.extend(
						frame(
							view,
							state,
							vec![
								egui::Event::PointerMoved(pos),
								egui::Event::PointerButton {
									pos,
									button: egui::PointerButton::Primary,
									pressed,
									modifiers: egui::Modifiers::NONE,
								},
							],
						)
						.1,
					);
				}
				commands
			};
			for _ in 0..3 {
				frame(&mut view, &mut state, vec![]);
			}
			assert_eq!(state.read_marker(Id(10)), Some(marker));
			let commands = click(&mut view, &mut state, "Jump to unread");
			assert!(commands.iter().any(|command| matches!(command,
                Command::History { channel: Id(10), before: None, after: Some(after), .. } if *after == marker.unwrap_or(Id(0))
            )));
			let (labels, _) = frame(&mut view, &mut state, vec![]);
			assert!(!labels.iter().any(|(text, _)| text == "Next messages"));
			for (first, last) in [(11, 15), (16, 20)] {
				let messages = (first..=last)
					.map(|id| {
						let mut message = message.clone();
						message.id = Id(id);
						message
					})
					.collect();
				state.apply(client_core::Envelope {
					generation: state.generation,
					event: client_core::Event::History {
						channel: Id(10),
						request: state.request,
						older: false,
						messages,
					},
				});
				for _ in 0..3 {
					frame(&mut view, &mut state, vec![]);
				}
				if last == 15 {
					// Older history has no bar of its own; this is the request its end sends.
					view.timeline.load_newer = true;
					let (_, commands) = frame(&mut view, &mut state, vec![]);
					assert!(commands.iter().any(|command| matches!(
						command,
						Command::History {
							before: None,
							after: Some(Id(15)),
							..
						}
					)));
				}
			}
			let (labels, _) = frame(&mut view, &mut state, vec![]);
			assert!(!labels.iter().any(|(text, _)| text == "Next messages"));
			assert_eq!(state.read_marker(Id(10)), Some(marker));
			let commands = click(&mut view, &mut state, "Jump to present");
			assert!(commands.iter().any(|command| matches!(
				command,
				Command::History {
					before: None,
					after: None,
					..
				}
			)));
			assert_eq!(state.drafts, drafts);
		}
	}

	#[test]
	fn reply_controls_are_inert_when_deleted_stale_or_unavailable() {
		use crate::test_ui::logical_text as collect;
		for blocked in [0, 1, 3, 4] {
			let ctx = egui::Context::default();
			let mut view = MessagingUi::default();
			let mut state = edit_state();
			let mut source = state.timeline.get(Id(20)).unwrap().clone();
			source.reply_to = Some(Id(19));
			source.reply_deleted = blocked == 0;
			source.kind = 19;
			state.timeline.insert(source.clone(), true, false).unwrap();
			state.reply = Some(client_core::Reply::to(Id(19)));
			match blocked {
				0 => {}
				1 => {
					state.timeline.delete(Id(19)).unwrap();
				}
				2 => {
					state.freshness = Freshness::Loading;
					state.history_pending = true;
				}
				3 => {
					state.freshness = Freshness::Stale;
				}
				4 => {
					state.channels.clear();
					state.freshness = Freshness::Unavailable;
				}
				_ => unreachable!(),
			}
			let draft = state.drafts.clone();
			let request = state.request;
			let frame = |view: &mut MessagingUi, state: &mut State, events| {
				let mut commands = vec![];
				let output = ctx.run_ui(
					egui::RawInput {
						screen_rect: Some(egui::Rect::from_min_size(
							egui::Pos2::ZERO,
							egui::vec2(760.0, 650.0),
						)),
						events,
						..Default::default()
					},
					|ui| commands = view.show(ui, state),
				);
				// egui reports settled text selection for the native PRIMARY clipboard.
				// Explicit clipboard writes and browser actions must remain absent.
				assert!(
					output
						.platform_output
						.commands
						.iter()
						.all(|command| matches!(
							command,
							egui::OutputCommand::TextSelectionSettled(_)
						)),
					"Unexpected platform actions: {:?}",
					output.platform_output.commands
				);
				assert!(!commands.iter().any(|command| matches!(
					command,
					Command::History { .. }
						| Command::Send { .. }
						| Command::Edit { .. }
						| Command::Delete { .. }
						| Command::MarkRead { .. }
						| Command::MarkGuildRead { .. }
				)));
				let mut labels = vec![];
				for shape in &output.shapes {
					collect(&shape.shape, &mut labels);
				}
				output.drop_without_applying_deltas();
				labels
			};
			for _ in 0..3 {
				frame(&mut view, &mut state, vec![]);
			}
			let labels = frame(&mut view, &mut state, vec![]);
			if blocked < 2 {
				assert!(labels.iter().any(|(text, _)| text == "Message deleted"));
			}
			// Activate the visible disabled controls (and the inert deleted label) with real input.
			for (_, rect) in labels.iter().filter(|(text, _)| {
				text == "View original"
					|| text == "Message deleted"
					|| text.starts_with("@Alex  ")
					|| text.starts_with("Earlier message")
			}) {
				let pos = rect.center();
				for pressed in [true, false] {
					frame(
						&mut view,
						&mut state,
						vec![
							egui::Event::PointerMoved(pos),
							egui::Event::PointerButton {
								pos,
								button: egui::PointerButton::Primary,
								pressed,
								modifiers: egui::Modifiers::NONE,
							},
						],
					);
				}
			}
			assert_eq!(state.request, request);
			assert!(state.search_target.is_none());
			assert_eq!(state.drafts, draft);
			assert_eq!(state.reply_target(), Some(Id(19)));
		}
	}

	#[test]
	fn reply_original_buttons_navigate_locally_or_request_one_page_without_sending() {
		use crate::test_ui::logical_text as collect;
		for (loaded, composer, keyboard, width) in [
			(true, false, false, 900.0),
			(false, false, true, 760.0),
			(true, true, true, 900.0),
			(false, true, false, 760.0),
		] {
			let ctx = egui::Context::default();
			let mut view = MessagingUi::default();
			let mut state = edit_state();
			let mut source = state.timeline.get(Id(20)).unwrap().clone();
			source.content = "Reply source".into();
			source.reply_to = Some(Id(19));
			state.timeline.insert(source.clone(), true, false).unwrap();
			if loaded {
				source.id = Id(19);
				source.reply_to = None;
				source.content = "||Hidden original||".into();
				state.timeline.insert(source, false, false).unwrap();
			}
			state.reply = Some(client_core::Reply::to(Id(19)));
			let draft = state.drafts.clone();
			let frame = |view: &mut MessagingUi, state: &mut State, events| {
				let mut commands = vec![];
				let output = ctx.run_ui(
					egui::RawInput {
						screen_rect: Some(egui::Rect::from_min_size(
							egui::Pos2::ZERO,
							egui::vec2(width, 650.0),
						)),
						events,
						..Default::default()
					},
					|ui| commands.extend(view.show(ui, state)),
				);
				// egui reports settled text selection for the native PRIMARY clipboard.
				// Explicit clipboard writes and browser actions must remain absent.
				assert!(
					output
						.platform_output
						.commands
						.iter()
						.all(|command| matches!(
							command,
							egui::OutputCommand::TextSelectionSettled(_)
						)),
					"Unexpected platform actions: {:?}",
					output.platform_output.commands
				);
				assert!(!commands.iter().any(|command| matches!(
					command,
					Command::Send { .. }
						| Command::Edit { .. }
						| Command::Delete { .. }
						| Command::MarkRead { .. }
						| Command::MarkGuildRead { .. }
				)));
				let mut labels = vec![];
				for shape in &output.shapes {
					collect(&shape.shape, &mut labels);
				}
				for event in &output.platform_output.events {
					if let egui::output::OutputEvent::FocusGained(info) = event
						&& let Some(label) = &info.label
						&& let Some(response) = ctx
							.memory(|m| m.focused())
							.and_then(|id| ctx.read_response(id))
					{
						labels.push((label.clone(), response.rect));
					}
				}
				output.drop_without_applying_deltas();
				(labels, commands)
			};
			for _ in 0..3 {
				frame(&mut view, &mut state, vec![]);
			}
			let label_matches = |text: &str| {
				if composer {
					text == "View original"
				} else if loaded {
					text == "@Alex  Spoiler"
				} else {
					text == "Earlier message \u{b7} View original"
						|| text == "Loading earlier message…"
				}
			};
			let (labels, _) = frame(&mut view, &mut state, vec![]);
			assert!(
				!labels
					.iter()
					.any(|(text, _)| text.contains("Hidden original"))
			);
			let request = state.request;
			let commands = if keyboard {
				ctx.memory_mut(|memory| {
					if let Some(id) = memory.focused() {
						memory.surrender_focus(id);
					}
				});
				frame(&mut view, &mut state, vec![egui::Event::PointerGone]);
				let key = |key| egui::Event::Key {
					key,
					physical_key: None,
					pressed: true,
					repeat: false,
					modifiers: egui::Modifiers::NONE,
				};
				let mut activated = None;
				for _ in 0..60 {
					let (labels, _) = frame(&mut view, &mut state, vec![key(egui::Key::Tab)]);
					if ctx
						.memory(|memory| memory.focused())
						.and_then(|id| ctx.read_response(id))
						.is_some_and(|response| {
							response.rect.height() < 36.0
								&& labels.iter().any(|(text, rect)| {
									label_matches(text) && response.rect.contains(rect.center())
								})
						}) {
						activated =
							Some(frame(&mut view, &mut state, vec![key(egui::Key::Enter)]).1);
						break;
					}
				}
				activated.expect("Tab must reach the original-message button")
			} else {
				let point = labels
					.iter()
					.find(|(text, _)| label_matches(text))
					.unwrap()
					.1
					.center();
				let mut activated = vec![];
				for pressed in [true, false] {
					activated.extend(
						frame(
							&mut view,
							&mut state,
							vec![
								egui::Event::PointerMoved(point),
								egui::Event::PointerButton {
									pos: point,
									button: egui::PointerButton::Primary,
									pressed,
									modifiers: egui::Modifiers::NONE,
								},
							],
						)
						.1,
					);
				}
				activated
			};
			assert_eq!(state.reply_target(), Some(Id(19)));
			assert_eq!(state.drafts, draft);
			assert!(view.draft_changes.is_empty());
			if loaded {
				assert_eq!(state.search_target, Some(Id(19)));
				assert_eq!(state.request, request);
				assert!(
					!commands
						.iter()
						.any(|command| matches!(command, Command::History { .. }))
				);
				for _ in 0..3 {
					frame(&mut view, &mut state, vec![]);
				}
				assert!(state.search_target.is_none());
			} else {
				assert!(
					state.search_target.is_none(),
					"A pending remote jump must not change the visible window"
				);
				assert_eq!(
					commands
						.iter()
						.filter(|command| matches!(
							command,
							Command::HistoryAround {
								channel: Id(10),
								target: Id(19),
								..
							}
						))
						.count(),
					1
				);
				assert!(state.history_pending);
				assert_eq!(
					state.freshness,
					Freshness::Fresh,
					"The source window stays usable during a remote jump"
				);
				for _ in 0..3 {
					let (_, commands) = frame(&mut view, &mut state, vec![]);
					assert!(
						!commands
							.iter()
							.any(|command| matches!(command, Command::History { .. }))
					);
				}
				state.apply(client_core::Envelope {
					generation: state.generation,
					event: client_core::Event::HistoryFailed {
						channel: Id(10),
						request: state.request,
						failure: client_core::auth::Failure::Network,
					},
				});
				let failure = state.status;
				let (_, commands) = frame(&mut view, &mut state, vec![]);
				assert_eq!(
					state.status, failure,
					"Rendering must preserve the actual request error"
				);
				assert!(
					!commands
						.iter()
						.any(|command| matches!(command, Command::History { .. }))
				);
				assert_eq!(state.drafts, draft);
				ctx.memory_mut(|memory| {
					if let Some(id) = memory.focused() {
						memory.surrender_focus(id);
					}
				});
				let mut reload = None;
				for _ in 0..80 {
					let (labels, _) = frame(&mut view, &mut state, vec![edit_key(egui::Key::Tab)]);
					if let Some(response) = ctx
						.memory(|memory| memory.focused())
						.and_then(|id| ctx.read_response(id))
						&& labels
							.iter()
							.any(|(text, rect)| text == "Reload history" && *rect == response.rect)
					{
						// Focus tooltips have their own text rectangles outside the icon.
						// Activate the actual button, not that painted tooltip.
						reload = Some(response.rect.center());
						break;
					}
				}
				let pos = reload.expect("Tab must reach Reload history");
				let mut reloads = 0;
				for pressed in [true, false] {
					let (_, commands) = frame(
						&mut view,
						&mut state,
						vec![
							egui::Event::PointerMoved(pos),
							egui::Event::PointerButton {
								pos,
								button: egui::PointerButton::Primary,
								pressed,
								modifiers: egui::Modifiers::NONE,
							},
						],
					);
					reloads += commands
						.iter()
						.filter(|command| {
							matches!(
								command,
								Command::History {
									channel: Id(10),
									before: None,
									..
								}
							)
						})
						.count();
				}
				assert_eq!(reloads, 1, "Explicit Reload returns to recent history");
				assert!(state.history_pending);
				assert_eq!(state.freshness, Freshness::Loading);
				assert!(!state.history_targeted);
				assert!(state.history_before.is_none() && state.history_after.is_none());
			}
		}
	}

	#[test]
	fn deleted_edit_target_keeps_user_changes_but_releases_untouched_original() {
		use crate::test_ui::logical_text as collect;
		let shortcut = |key| egui::Event::Key {
			key,
			physical_key: None,
			pressed: true,
			repeat: false,
			modifiers: egui::Modifiers::COMMAND,
		};
		for (modified, reopen) in [(false, false), (true, false), (true, true)] {
			let ctx = egui::Context::default();
			let mut state = edit_state();
			let mut view = MessagingUi {
				editing: Some((Id(10), Id(20), "Original".into())),
				..Default::default()
			};
			let frame = |view: &mut MessagingUi, state: &mut State, events| {
				let mut commands = vec![];
				let output = ctx.run_ui(
					egui::RawInput {
						screen_rect: Some(egui::Rect::from_min_size(
							egui::Pos2::ZERO,
							egui::vec2(900.0, 650.0),
						)),
						events,
						..Default::default()
					},
					|ui| commands = view.show(ui, state),
				);
				assert!(!commands.iter().any(|command| matches!(
					command,
					Command::Send { .. } | Command::Edit { .. } | Command::Delete { .. }
				)));
				assert!(
					output
						.platform_output
						.commands
						.iter()
						.all(|command| matches!(
							command,
							egui::OutputCommand::TextSelectionSettled(_)
						))
				);
				let mut labels = vec![];
				for shape in &output.shapes {
					collect(&shape.shape, &mut labels);
				}
				for event in &output.platform_output.events {
					if let egui::output::OutputEvent::FocusGained(info) = event
						&& let Some(label) = &info.label
						&& let Some(response) = ctx
							.memory(|m| m.focused())
							.and_then(|id| ctx.read_response(id))
					{
						labels.push((label.clone(), response.rect));
					}
				}
				output.drop_without_applying_deltas();
				labels
			};
			for _ in 0..3 {
				frame(&mut view, &mut state, vec![]);
			}
			if modified {
				frame(
					&mut view,
					&mut state,
					vec![
						shortcut(egui::Key::A),
						egui::Event::Text("Only my unsent text".into()),
					],
				);
				assert_eq!(view.editing.as_ref().unwrap().2, "Only my unsent text");
			}
			if reopen {
				let labels = frame(&mut view, &mut state, vec![]);
				let message = labels
					.iter()
					.find(|(text, _)| text.trim_end() == "Original")
					.expect("Loaded message remains visible")
					.1
					.center();
				frame(
					&mut view,
					&mut state,
					vec![egui::Event::PointerMoved(message)],
				);
				ctx.memory_mut(|memory| {
					if let Some(id) = memory.focused() {
						memory.surrender_focus(id);
					}
				});
				let mut activated = false;
				for _ in 0..80 {
					let labels = frame(&mut view, &mut state, vec![edit_key(egui::Key::Tab)]);
					if labels.iter().any(|(text, _)| text == "Edit Message") {
						frame(&mut view, &mut state, vec![edit_key(egui::Key::Enter)]);
						activated = true;
						break;
					}
				}
				assert!(activated, "Tab must reach the own-message edit action");
				assert_eq!(view.editing.as_ref().unwrap().2, "Original");
			}
			let retained = view.editing.as_ref().unwrap().2.clone();
			let keep_edit = modified && !reopen;
			assert_eq!(retained != "Original", keep_edit);
			view.deleting = Some((Id(10), Id(20)));
			state.timeline.delete(Id(20)).unwrap();
			state.revision += 1;
			view.messages_deleted(&ctx, Id(10), &[Id(20)]);
			frame(&mut view, &mut state, vec![edit_key(egui::Key::Enter)]);
			assert!(view.deleting.is_none());
			assert_eq!(view.has_edit(), keep_edit);
			if keep_edit {
				frame(&mut view, &mut state, vec![shortcut(egui::Key::Z)]);
				assert_eq!(
					view.editing.as_ref().unwrap().2,
					retained,
					"Undo cannot restore the deleted original"
				);
				let labels = frame(&mut view, &mut state, vec![]);
				assert_eq!(view.editing.as_ref().unwrap().2, retained);
				assert!(labels.iter().any(|(label, _)| label == "Copy edit text"));
				assert!(
					labels
						.iter()
						.any(|(label, _)| label.contains("Message unavailable"))
				);
				frame(&mut view, &mut state, vec![edit_key(egui::Key::Escape)]);
				assert!(!view.has_edit());
			}
			assert_eq!(state.drafts[&Id(10)], "Unsent draft 👋");
			assert!(view.draft_changes.is_empty());
		}
	}

	#[test]
	fn off_channel_deletion_releases_original_and_undo_without_touching_other_input() {
		for modified in [false, true] {
			let ctx = egui::Context::default();
			let mut state = edit_state();
			let mut view = MessagingUi {
				editing: Some((Id(10), Id(20), "Original".into())),
				..Default::default()
			};
			for _ in 0..3 {
				edit_frame(&ctx, &mut view, &mut state, vec![]);
			}
			if modified {
				let select_all = egui::Event::Key {
					key: egui::Key::A,
					physical_key: None,
					pressed: true,
					repeat: false,
					modifiers: egui::Modifiers::COMMAND,
				};
				edit_frame(
					&ctx,
					&mut view,
					&mut state,
					vec![select_all, egui::Event::Text("User replacement".into())],
				);
			}
			let text = view.editing.as_ref().unwrap().2.clone();
			let widget = view.edit_widget_id.unwrap();
			let cursor = egui::text_edit::TextEditState::load(&ctx, widget)
				.unwrap()
				.cursor
				.char_range()
				.unwrap();
			state.selected = Some(Id(11));
			state.channels.push(model::Channel {
				id: Id(11),
				guild: None,
				kind: 1,
				name: "Other conversation".into(),
				last_message: None,
				parent_id: None,
				position: 0,
				recipients: vec![],
				member_list_id: None,
				tags: None,
				message_count: None,
				icon: None,
			});
			edit_frame(&ctx, &mut view, &mut state, vec![]);
			view.deleting = Some((Id(10), Id(20)));
			for (channel, ids) in [
				(Id(11), vec![Id(20)]),
				(Id(10), vec![Id(21)]),
				(Id(10), vec![Id(20); 101]),
			] {
				view.messages_deleted(&ctx, channel, &ids);
				assert_eq!(view.editing.as_ref().unwrap().2, text);
				assert!(view.deleting.is_some());
			}
			view.messages_deleted(&ctx, Id(10), &[Id(20)]);
			assert!(view.deleting.is_none());
			assert_eq!(view.has_edit(), modified);
			let editor = egui::text_edit::TextEditState::load(&ctx, widget).unwrap();
			assert!(
				editor.undoer().undo(&(cursor, text.clone())).is_none(),
				"Original undo snapshots must be gone before revisiting the conversation"
			);
			if modified {
				assert_eq!(view.editing.as_ref().unwrap().2, "User replacement");
				assert_eq!(editor.cursor.char_range(), Some(cursor));
			}
			// The close notification is scoped to A; B must still accept this frame's input.
			let output = ctx.run_ui(
				egui::RawInput {
					events: vec![egui::Event::Text("B input".into())],
					..Default::default()
				},
				|ui| {
					ctx.memory_mut(|memory| {
						memory.request_focus(ui.make_persistent_id("message-input"))
					});
					view.composer(ui, &mut state, Id(11), &ctx, &mut vec![]);
				},
			);
			output.drop_without_applying_deltas();
			assert_eq!(state.drafts[&Id(11)], "B input");
			assert_eq!(state.drafts[&Id(10)], "Unsent draft 👋");
		}
	}

	#[test]
	fn member_pane_virtualizes_and_preview_never_requests_network() {
		fn collect_text(shape: &egui::Shape, text: &mut Vec<String>) {
			match shape {
				egui::Shape::Text(shape) => text.push(shape.galley.job.text.clone()),
				egui::Shape::Vec(shapes) => {
					for shape in shapes {
						collect_text(shape, text);
					}
				}
				_ => {}
			}
		}

		let mut state = State {
			demo: true,
			selected: Some(Id(1)),
			channels: vec![model::Channel {
				last_message: None,
				id: Id(1),
				guild: Some(Id(2)),
				parent_id: None,
				position: 0,
				name: "Synthetic".into(),
				kind: 0,
				recipients: Vec::new(),
				member_list_id: Some("everyone".into()),
				tags: None,
				message_count: None,
				icon: None,
			}],
			members: Some(model::MemberList {
				missing_identity: vec![],
				channel: Id(1),
				guild: Some(Id(2)),
				request: 1,
				total: 100,
				lazy: false,
				groups: vec![
					("8".into(), 2),
					("online".into(), 1),
					("offline".into(), 97),
				],
				ranges: vec![],
				freshness: Freshness::Fresh,
				start: 0,
				slots: (1..=100)
					.map(|id| {
						Some(model::MemberSlot::Person(model::Member {
							user: model::User {
								member_avatar: None,
								username: None,
								id: Id(id),
								name: format!("Synthetic {id}"),
								avatar: None,
								webhook: false,
								kind: Default::default(),
								discriminator: 0,
								primary_guild: None,
								cosmetics: None,
							},
							nick: None,
							roles: vec![],
							status: match id {
								1 => Some("online"),
								2 => Some("idle"),
								3 => Some("dnd"),
								4 => Some("offline"),
								5 => Some("unknown"),
								_ => None,
							}
							.map(str::to_owned),
							custom_status: None,
							activities: vec![],
							clients: model::ClientPlatforms::default(),
						}))
					})
					.collect(),
			}),
			..Default::default()
		};
		state.permissions.guilds.insert(
			Id(2),
			model::permissions::Guild {
				id: Id(2),
				owner: None,
				member: None,
				roles: Some(vec![model::permissions::Role {
					icon: None,
					unicode_emoji: None,
					id: Id(8),
					bits: 0,
					name: "Founders".into(),
					color: 0xe78284,
					secondary_color: None,
					tertiary_color: None,
					position: 1,
					hoist: true,
				}]),
			},
		);
		for member in state
			.members
			.as_mut()
			.unwrap()
			.slots
			.iter_mut()
			.flatten()
			.filter_map(|slot| match slot {
				model::MemberSlot::Person(m) => Some(m),
				_ => None,
			})
			.take(2)
		{
			member.roles.push(Id(8));
		}
		let slots = &mut state.members.as_mut().unwrap().slots;
		if let Some(model::MemberSlot::Person(m)) = slots[0].as_mut() {
			m.user.kind = model::AccountKind::Bot;
		}
		if let Some(model::MemberSlot::Person(m)) = slots[1].as_mut() {
			m.user.kind = model::AccountKind::App;
		}
		if let Some(model::MemberSlot::Person(m)) = slots[2].as_mut() {
			m.user.webhook = true;
		}
		slots.insert(0, Some(model::MemberSlot::Group("offline".into())));
		slots.insert(0, Some(model::MemberSlot::Group("online".into())));
		slots.insert(0, Some(model::MemberSlot::Group("8".into())));
		let mut messaging = MessagingUi::default();
		let context = egui::Context::default();
		let output = context.run_ui(
			egui::RawInput {
				screen_rect: Some(egui::Rect::from_min_size(
					egui::Pos2::ZERO,
					egui::vec2(1120.0, 760.0),
				)),
				..Default::default()
			},
			|ui| {
				assert!(messaging.show(ui, &mut state).is_empty());
			},
		);
		assert!(messaging.take_avatar_requests().is_empty());
		let mut text = Vec::new();
		for shape in &output.shapes {
			collect_text(&shape.shape, &mut text);
		}
		for heading in [
			"BOT",
			"APP",
			"WEBHOOK",
			"Founders - 2",
			"Online - 1",
			"Offline - 97",
		] {
			assert!(
				text.iter().any(|label| label == heading),
				"Missing {heading}"
			);
		}
		for id in 1..=6 {
			assert!(text.iter().any(|label| label == &format!("Synthetic {id}")));
		}
		for status in [
			"Online",
			"Away",
			"Do not disturb",
			"Offline",
			"Presence unavailable",
		] {
			assert!(
				!text.iter().any(|label| label == status),
				"Members without activity or custom status must have no subtitle: {status}"
			);
		}
		assert!(
			!output.textures_delta.set.is_empty(),
			"Preview must exercise actual image uploads"
		);
		assert!(
			output.textures_delta.set.len() < 30,
			"Offscreen members must not upload textures"
		);
		output.drop_without_applying_deltas();
	}

	#[test]
	fn group_dm_member_count_tracks_participants_and_unavailable_states() {
		fn collect(shape: &egui::Shape, labels: &mut Vec<String>) {
			match shape {
				egui::Shape::Text(text) => labels.push(text.galley.job.text.clone()),
				egui::Shape::Vec(shapes) => {
					for shape in shapes {
						collect(shape, labels);
					}
				}
				_ => {}
			}
		}
		let mut state = test_support::demo_state();
		let channel = Id(29);
		state.selected = Some(channel);
		let _ = state.request_members();
		let ctx = egui::Context::default();
		let mut view = MessagingUi::default();
		let render = |view: &mut MessagingUi, state: &mut State, width| {
			let output = ctx.run_ui(
				egui::RawInput {
					screen_rect: Some(egui::Rect::from_min_size(
						egui::Pos2::ZERO,
						egui::vec2(width, 400.0),
					)),
					..Default::default()
				},
				|ui| view.member_rows(ui, state, &mut vec![]),
			);
			let mut labels = vec![];
			for shape in &output.shapes {
				collect(&shape.shape, &mut labels);
			}
			output.drop_without_applying_deltas();
			labels
		};
		assert_eq!(state.channel(channel).unwrap().recipients.len(), 2);
		assert!(render(&mut view, &mut state, 240.0).contains(&"Members — 3".into()));
		view.language = i18n::Language::Czech;
		assert!(render(&mut view, &mut state, 240.0).contains(&"Členové — 3".into()));
		view.language = i18n::Language::English;
		let mut user = test_support::message(9, channel).author;
		user.id = Id(90001);
		state.apply(client_core::Envelope {
			generation: state.generation,
			event: client_core::Event::RecipientAdded {
				channel,
				user: user.clone(),
			},
		});
		assert!(render(&mut view, &mut state, 340.0).contains(&"Members — 4".into()));
		state.apply(client_core::Envelope {
			generation: state.generation,
			event: client_core::Event::RecipientRemoved {
				channel,
				user: user.id,
			},
		});
		assert!(render(&mut view, &mut state, 240.0).contains(&"Members — 3".into()));
		// A recipient snapshot containing the current account must not count it twice.
		state.apply(client_core::Envelope {
			generation: state.generation,
			event: client_core::Event::RecipientAdded {
				channel,
				user: state.user.clone().unwrap(),
			},
		});
		assert!(render(&mut view, &mut state, 240.0).contains(&"Members — 3".into()));
		for freshness in [Freshness::Loading, Freshness::Unavailable, Freshness::Fresh] {
			let list = state.members.as_mut().unwrap();
			list.slots.clear();
			list.total = 0;
			list.freshness = freshness;
			let labels = render(&mut view, &mut state, 240.0);
			assert_eq!(
				labels.contains(&"Members — 0".into()),
				freshness == Freshness::Fresh
			);
		}
		state.selected = Some(Id(22)); // A one-to-one DM keeps its existing presentation.
		let _ = state.request_members();
		assert!(
			!render(&mut view, &mut state, 240.0)
				.iter()
				.any(|label| label.starts_with("Members —"))
		);
	}

	#[test]
	fn incoming_custom_status_updates_people_and_open_profile_without_refetch() {
		fn collect(shape: &egui::Shape, labels: &mut Vec<String>) {
			match shape {
				egui::Shape::Text(text) => labels.push(text.galley.job.text.clone()),
				egui::Shape::Vec(shapes) => {
					for shape in shapes {
						collect(shape, labels);
					}
				}
				_ => {}
			}
		}
		let mut state = test_support::demo_state();
		state.demo = false; // Exercise normal command admission using synthetic loaded data.
		state.guild_folders = Some(Default::default()); // Folder fetch is outside this presence-only scenario.
		// Forum sidebar loading is outside this presence-only scenario.
		state
			.channels
			.retain(|entry| !matches!(entry.kind, 15 | 16));
		let channel = state.selected.unwrap();
		let guild = state
			.channels
			.iter()
			.find(|entry| entry.id == channel)
			.unwrap()
			.guild
			.unwrap();
		// Sidebar forum discovery is outside this presence-only scenario.
		state.channels.retain(|entry| entry.id == channel);
		state.invalidate_navigation();
		let user = test_support::message(499, channel).author;
		// No read acknowledgement is part of this presence-only scenario: clear the page and
		// the latest-message metadata that an empty live edge would otherwise acknowledge.
		state.timeline.clear();
		if let Some(channel) = state.channels.iter_mut().find(|c| c.id == channel) {
			channel.last_message = None;
		}
		state.members = Some(model::MemberList {
			missing_identity: vec![],
			channel,
			guild: Some(guild),
			request: 7,
			total: 1,
			lazy: false,
			groups: vec![],
			ranges: vec![],
			freshness: Freshness::Fresh,
			start: 0,
			slots: vec![Some(model::MemberSlot::Person(model::Member {
				user: user.clone(),
				nick: None,
				roles: vec![],
				status: Some("online".into()),
				custom_status: Some("Initial synthetic status".into()),
				activities: vec![],
				clients: model::ClientPlatforms::default(),
			}))],
		});
		state.profile = Some(client_core::profile::ProfileView {
			user: user.id,
			guild: Some(guild),
			request: 314,
			loading: false,
			error: None,
			data: Some(profiles::synthetic(&user, Some(guild))),
		});
		// The card's one private-note read is outside this presence-only scenario.
		let Some(Command::UserAction { request, .. }) = state.load_user_note(user.id) else {
			panic!("The synthetic profile note loads");
		};
		state.apply(client_core::Envelope {
			generation: state.generation,
			event: client_core::Event::UserAction(client_core::user_actions::Event::NoteLoaded {
				user: user.id,
				request,
				result: Ok(String::new()),
			}),
		});
		assert_eq!(state.user_note(user.id), Some(""));
		state
			.drafts
			.insert(channel, "Keep this unsent draft".into());
		let mut messaging = MessagingUi {
			navigation_channel: Some(channel),
			guild: Some(guild),
			profile: {
				let mut profile = profiles::ProfileSession::default();
				profile.command_open(user.clone());
				profile
			},
			..Default::default()
		};
		let ctx = egui::Context::default();
		design::apply(&ctx);
		let frame = |messaging: &mut MessagingUi, state: &mut State| {
			let mut commands = vec![];
			let output = ctx.run_ui(
				egui::RawInput {
					screen_rect: Some(egui::Rect::from_min_size(
						egui::Pos2::ZERO,
						egui::vec2(1280.0, 900.0),
					)),
					..Default::default()
				},
				|ui| commands = messaging.show(ui, state),
			);
			let platform_commands_empty = output.platform_output.commands.is_empty();
			let mut labels = vec![];
			for shape in &output.shapes {
				collect(&shape.shape, &mut labels);
			}
			output.drop_without_applying_deltas();
			assert!(
				commands.is_empty(),
				"Presence rendering must not fetch a profile or emit other commands"
			);
			assert!(platform_commands_empty);
			labels
		};
		for _ in 0..3 {
			frame(&mut messaging, &mut state);
		}
		let labels = frame(&mut messaging, &mut state);
		assert_eq!(
			labels
				.iter()
				.filter(|text| text.as_str() == "Initial synthetic status")
				.count(),
			2,
			"People and the already-open profile both show the loaded status"
		);
		for custom_status in [Some("Updated synthetic status"), None] {
			state.apply(client_core::Envelope {
				generation: state.generation,
				event: client_core::Event::MemberPresence {
					guild,
					channel,
					request: 7,
					updates: vec![model::MemberPresence {
						user: user.id,
						status: Some("online".into()),
						custom_status: custom_status.map(str::to_owned),
						activities: vec![],
						clients: model::ClientPlatforms::default(),
					}],
				},
			});
			for _ in 0..2 {
				frame(&mut messaging, &mut state);
			}
			let labels = frame(&mut messaging, &mut state);
			assert!(!labels.iter().any(|text| text == "Initial synthetic status"));
			assert_eq!(
				labels
					.iter()
					.filter(|text| text.as_str() == "Updated synthetic status")
					.count(),
				if custom_status.is_some() { 2 } else { 0 }
			);
			assert_eq!(state.profile.as_ref().unwrap().request, 314);
			assert_eq!(messaging.profile.open_user().unwrap().id, user.id);
			assert_eq!(state.drafts[&channel], "Keep this unsent draft");
			assert!(messaging.draft_changes.is_empty());
		}
	}

	#[test]
	fn rich_presence_reaches_lists_dm_header_and_profile_then_clears() {
		fn collect(shape: &egui::Shape, output: &mut String) {
			match shape {
				egui::Shape::Text(text) => {
					output.push_str(&text.galley.job.text);
					output.push('\n');
				}
				egui::Shape::Vec(shapes) => {
					for shape in shapes {
						collect(shape, output);
					}
				}
				_ => {}
			}
		}
		for dm in [false, true] {
			let mut state = test_support::demo_state();
			let user = test_support::message(1, Id(22)).author;
			let channel = if dm { Id(22) } else { state.selected.unwrap() };
			let _ = state.select(channel);
			state.timeline.clear();
			state.history_pending = false;
			let guild = state
				.channels
				.iter()
				.find(|c| c.id == channel)
				.unwrap()
				.guild;
			let _ = state.request_members();
			state.members = Some(model::MemberList {
				missing_identity: vec![],
				guild,
				channel,
				request: 7,
				total: 1,
				lazy: false,
				groups: vec![],
				ranges: vec![],
				freshness: Freshness::Fresh,
				start: 0,
				slots: vec![Some(model::MemberSlot::Person(model::Member {
					roles: vec![],
					user: user.clone(),
					nick: None,
					status: Some("online".into()),
					custom_status: None,
					activities: vec![],
					clients: model::ClientPlatforms::default(),
				}))],
			});
			let mut messaging = MessagingUi {
				navigation_channel: Some(channel),
				guild,
				profile: {
					let mut profile = profiles::ProfileSession::default();
					profile.command_open(user.clone());
					profile
				},
				..Default::default()
			};
			let ctx = egui::Context::default();
			design::apply(&ctx);
			for clear in [false, true] {
				let activities = if clear {
					vec![]
				} else {
					vec![model::RichActivity {
						kind: 0,
						name: "Stardew Valley".into(),
						details: Some("Tending the farm".into()),
						state: Some("Spring Day 12".into()),
						image: Some(model::ActivityImage::Asset {
							application: Id(9001),
							asset: Id(9002),
						}),
						small_image: None,
						ends_at: None,
						started_at: None,
					}]
				};
				let event = if let Some(guild) = guild {
					client_core::Event::MemberPresence {
						guild,
						channel,
						request: 7,
						updates: vec![model::MemberPresence {
							user: user.id,
							status: Some("online".into()),
							custom_status: None,
							activities,
							clients: model::ClientPlatforms::default(),
						}],
					}
				} else {
					client_core::Event::DirectPresence(vec![client_core::presence::Update {
						user: user.id,
						status: model::Patch::Value("online".into()),
						activities: model::Patch::Value(activities),
						custom_status: model::Patch::Absent,
						clients: model::Patch::Absent,
					}])
				};
				state.apply(client_core::Envelope {
					generation: state.generation,
					event,
				});
				let mut painted = String::new();
				let mut artwork = Vec::new();
				for _ in 0..3 {
					painted.clear();
					artwork.clear();
					let output = ctx.run_ui(
						egui::RawInput {
							screen_rect: Some(egui::Rect::from_min_size(
								egui::Pos2::ZERO,
								egui::vec2(1280.0, 900.0),
							)),
							..Default::default()
						},
						|ui| {
							messaging.show(ui, &mut state);
						},
					);
					assert!(
						output
							.platform_output
							.commands
							.iter()
							.all(|command| matches!(
								command,
								egui::OutputCommand::TextSelectionSettled(_)
							))
					);
					for shape in &output.shapes {
						collect(&shape.shape, &mut painted);
					}
					// Match this activity's actual texture, not unrelated same-sized UI meshes.
					let activity_texture = messaging.avatars.texture_id(
						&model::ActivityImage::Asset {
							application: Id(9001),
							asset: Id(9002),
						}
						.key(),
					);
					artwork = ctx
						.tessellate(output.shapes.clone(), output.pixels_per_point)
						.iter()
						.filter_map(|shape| match &shape.primitive {
							egui::epaint::Primitive::Mesh(mesh)
								if Some(mesh.texture_id) == activity_texture
									&& shape
										.clip_rect
										.intersect(mesh.calc_bounds())
										.is_positive() =>
							{
								Some(mesh.calc_bounds())
							}
							_ => None,
						})
						.collect();
					output.drop_without_applying_deltas();
				}
				assert_eq!(
					painted.matches("Playing Stardew Valley").count(),
					if clear {
						0
					} else if dm {
						3
					} else {
						1
					},
					"{painted}"
				);
				assert_eq!(painted.contains("Tending the farm"), !clear);
				assert_eq!(painted.contains("Spring Day 12"), !clear);
				assert_eq!(
					artwork.len(),
					usize::from(!clear),
					"Activity image appears and clears with its presence"
				);
			}
		}
	}

	#[test]
	fn startup_warning_is_visible_then_clears_on_ready_and_logout() {
		fn contains_warning(shape: &egui::Shape) -> bool {
			match shape {
				egui::Shape::Text(text) => text.galley.job.text.contains(
					"Connected with some data unavailable: read status, notification settings",
				),
				egui::Shape::Vec(shapes) => shapes.iter().any(contains_warning),
				_ => false,
			}
		}
		let ctx = egui::Context::default();
		design::apply(&ctx);
		let mut state = test_support::demo_state();
		let mut messaging = MessagingUi::default();
		let visible = |messaging: &mut MessagingUi, state: &mut State| {
			let output = ctx.run_ui(
				egui::RawInput {
					screen_rect: Some(egui::Rect::from_min_size(
						egui::Pos2::ZERO,
						egui::vec2(1120.0, 900.0),
					)),
					..Default::default()
				},
				|ui| {
					messaging.show(ui, state);
				},
			);
			let visible = output
				.shapes
				.iter()
				.any(|shape| contains_warning(&shape.shape));
			output.drop_without_applying_deltas();
			visible
		};
		for logout in [false, true] {
			state.apply(client_core::Envelope {
				generation: state.generation,
				event: client_core::Event::StartupWarnings(model::account::Warnings {
					read_state: true,
					notifications: true,
					..Default::default()
				}),
			});
			visible(&mut messaging, &mut state);
			assert!(visible(&mut messaging, &mut state));
			if logout {
				state.logout();
			} else {
				state.apply(client_core::Envelope {
					generation: state.generation,
					event: client_core::Event::Ready {
						user: state.user.clone().unwrap(),
						guilds: state.guilds.clone(),
						channels: state.channels.clone(),
						permissions: Default::default(),
					},
				});
			}
			assert!(!visible(&mut messaging, &mut state));
		}
	}

	#[test]
	fn webhook_profile_does_not_request_a_user_profile() {
		for webhook in [false, true] {
			let mut user = test_support::message(1, Id(22)).author;
			user.webhook = webhook;
			let mut state = State {
				auth: client_core::auth::AuthState::Authenticated,
				gateway_connected: true,
				..Default::default()
			};
			let mut messaging = MessagingUi {
				profile: {
					let mut profile = profiles::ProfileSession::default();
					profile.command_open(user);
					profile
				},
				..Default::default()
			};
			let mut commands = vec![];
			let output = egui::Context::default().run_ui(
				egui::RawInput {
					screen_rect: Some(egui::Rect::from_min_size(
						egui::Pos2::ZERO,
						egui::vec2(1120.0, 900.0),
					)),
					..Default::default()
				},
				|ui| commands = messaging.show(ui, &mut state),
			);
			output.drop_without_applying_deltas();
			assert_eq!(
				commands
					.iter()
					.any(|command| matches!(command, Command::Profile { .. })),
				!webhook
			);
			assert_eq!(state.profile.is_some(), !webhook);
		}
	}

	#[test]
	fn text_author_click_toggles_profile_like_voice() {
		use crate::test_ui::logical_text as labels;
		let ctx = egui::Context::default();
		let mut state = test_support::demo_state();
		let channel = state.selected.unwrap();
		state.apply(client_core::Envelope {
			generation: state.generation,
			event: client_core::Event::Message(test_support::message(501, channel)),
		});
		let mut messaging = MessagingUi::default();
		let frame = |messaging: &mut MessagingUi, state: &mut State, events: Vec<egui::Event>| {
			let mut output = ctx.run_ui(
				egui::RawInput {
					screen_rect: Some(egui::Rect::from_min_size(
						egui::Pos2::ZERO,
						egui::vec2(1120.0, 900.0),
					)),
					events,
					..Default::default()
				},
				|ui| {
					messaging.show(ui, state);
				},
			);
			output.textures_delta.clear();
			let mut text = vec![];
			for shape in output.shapes {
				labels(&shape.shape, &mut text);
			}
			text
		};
		let click = |messaging: &mut MessagingUi, state: &mut State, pos: egui::Pos2| {
			for pressed in [true, false] {
				frame(
					messaging,
					state,
					vec![
						egui::Event::PointerMoved(pos),
						egui::Event::PointerButton {
							pos,
							button: egui::PointerButton::Primary,
							pressed,
							modifiers: egui::Modifiers::NONE,
						},
					],
				);
			}
		};
		let text = frame(&mut messaging, &mut state, vec![]);
		let (_, author) = text
			.iter()
			.filter(|(label, _)| label == "Robin (synthetic)")
			.min_by(|(_, left), (_, right)| left.min.x.total_cmp(&right.min.x))
			.expect("message author");
		let pos = author.center();
		click(&mut messaging, &mut state, pos);
		assert_eq!(
			messaging.profile.open_user().map(|user| user.id),
			Some(Id(2))
		);
		click(&mut messaging, &mut state, pos);
		assert!(messaging.profile.open_user().is_none());
		assert!(
			state
				.profile
				.as_ref()
				.is_some_and(|view| view.data.is_some())
		);
	}

	#[test]
	fn profile_uses_open_conversation_not_browsed_sidebar_server() {
		for guild in [None, Some(Id(10))] {
			let user = model::User {
				member_avatar: None,
				username: None,
				id: Id(2),
				name: "Synthetic".into(),
				avatar: None,
				webhook: false,
				kind: Default::default(),
				discriminator: 0,
				primary_guild: None,
				cosmetics: None,
			};
			let mut state = State {
				demo: true,
				selected: Some(Id(1)),
				channels: vec![model::Channel {
					last_message: None,
					id: Id(1),
					guild,
					parent_id: None,
					position: 0,
					name: "Open conversation".into(),
					kind: if guild.is_some() { 0 } else { 1 },
					recipients: vec![user.clone()],
					member_list_id: None,
					tags: None,
					message_count: None,
					icon: None,
				}],
				..Default::default()
			};
			let mut messaging = MessagingUi {
				navigation_channel: state.selected,
				guild: Some(Id(99)),
				profile: {
					let mut profile = profiles::ProfileSession::default();
					profile.command_open(user);
					profile
				},
				..Default::default()
			};
			let output = egui::Context::default().run_ui(
				egui::RawInput {
					screen_rect: Some(egui::Rect::from_min_size(
						egui::Pos2::ZERO,
						egui::vec2(1120.0, 900.0),
					)),
					..Default::default()
				},
				|ui| {
					messaging.show(ui, &mut state);
				},
			);
			assert_eq!(state.profile.as_ref().unwrap().guild, guild);
			output.drop_without_applying_deltas();
		}
	}

	#[test]
	fn inline_edit_keeps_the_pending_draft_attachment_separate() {
		let ctx = egui::Context::default();
		let mut state = edit_state();
		let mut view = MessagingUi {
			editing: Some((Id(10), Id(20), "Replacement".into())),
			attachment: Some(("draft.txt".into(), 32)),
			upload_busy: true,
			..Default::default()
		};
		edit_frame(&ctx, &mut view, &mut state, vec![]);
		let commands = edit_frame(
			&ctx,
			&mut view,
			&mut state,
			vec![edit_key(egui::Key::Enter)],
		);
		assert!(
			matches!(commands.as_slice(), [Command::Edit { content, .. }] if content == "Replacement")
		);
		assert_eq!(state.drafts[&Id(10)], "Unsent draft 👋");
		assert_eq!(view.attachment, Some(("draft.txt".into(), 32)));
		assert!(
			!view.attach_requested
				&& !view.remove_attachment_requested
				&& !view.cancel_upload_requested
		);
	}

	#[test]
	fn pending_preview_waits_for_confirmation_and_restore_preserves_new_drafts() {
		let ctx = egui::Context::default();
		let mut state = test_support::demo_state();
		let channel = state.selected.unwrap();
		let mut view = MessagingUi::default();
		view.preview_attachment(
			"synthetic.png",
			100,
			Some(egui::ColorImage::filled([2, 2], egui::Color32::WHITE)),
		);
		view.preview_sending(&ctx, &mut state);
		let nonce = state.pending.last().unwrap().nonce.clone();
		assert!(
			view.pending_upload.as_ref().unwrap().files[0]
				.preview
				.is_some()
		);
		assert!(view.attachment.is_none());
		view.update_upload_progress(None, true);
		assert_eq!(
			view.pending_upload.as_ref().unwrap().progress,
			Some((100, 100))
		);
		assert_eq!(
			state.pending.last().unwrap().delivery,
			model::Delivery::Sending
		);
		state.pending.last_mut().unwrap().delivery = model::Delivery::Ambiguous;
		state.drafts.insert(channel, "New draft".into());
		view.restore_pending(&mut state, channel, &nonce);
		assert_eq!(state.drafts[&channel], "New draft");
		assert!(state.pending.iter().any(|p| p.nonce == nonce));
		state.drafts.remove(&channel);
		view.restore_pending(&mut state, channel, &nonce);
		assert!(state.drafts[&channel].contains("sending it now"));
		assert!(!state.pending.iter().any(|p| p.nonce == nonce));
		ctx.run_ui(Default::default(), |ui| {
			view.show(ui, &mut state);
		})
		.drop_without_applying_deltas();
		assert!(view.pending_upload.is_none());

		view.preview_attachment("synthetic.png", 100, None);
		view.preview_sending(&ctx, &mut state);
		let nonce = state.pending.last().unwrap().nonce.clone();
		let mut confirmed = test_support::message(999_999, channel);
		confirmed.author = state.user.clone().unwrap();
		confirmed.nonce = Some(nonce.clone());
		state.apply(client_core::Envelope {
			generation: state.generation,
			event: client_core::Event::SendResult {
				nonce,
				result: Ok(confirmed),
			},
		});
		ctx.run_ui(Default::default(), |ui| {
			view.show(ui, &mut state);
		})
		.drop_without_applying_deltas();
		assert!(view.pending_upload.is_none());
		assert!(state.timeline.get(Id(999_999)).is_some());
	}

	#[test]
	fn attachment_only_enter_sends_once_and_busy_upload_blocks_resending() {
		for (busy, allowed, modifiers, repeat) in [
			(true, true, egui::Modifiers::NONE, false),
			(false, true, egui::Modifiers::NONE, false),
			(false, false, egui::Modifiers::NONE, false),
			(false, true, egui::Modifiers::SHIFT, false),
			(false, true, egui::Modifiers::ALT, false),
			(false, true, egui::Modifiers::NONE, true),
		] {
			let ctx = egui::Context::default();
			let mut state = test_support::demo_state();
			state.demo = false;
			let channel = state.selected.unwrap();
			state.drafts.remove(&channel);
			if !allowed {
				state.channels.clear();
			}
			let mut messaging = MessagingUi {
				attachment: Some(("synthetic.txt".into(), 32)),
				upload_busy: busy,
				..Default::default()
			};
			let mut commands = Vec::new();
			let mut editor = egui::Id::NULL;
			ctx.run_ui(Default::default(), |ui| {
				editor = ui.make_persistent_id("message-input");
				messaging.composer(ui, &mut state, channel, &ctx, &mut commands);
			})
			.drop_without_applying_deltas();
			assert!(commands.is_empty(), "Selecting a file must not send it");
			ctx.memory_mut(|m| m.request_focus(editor));
			if repeat {
				// egui derives repeat from held keys, overriding the raw event flag.
				ctx.input_mut(|i| i.keys_down.insert(egui::Key::Enter));
			}
			ctx.run_ui(
				egui::RawInput {
					events: vec![egui::Event::Key {
						key: egui::Key::Enter,
						physical_key: None,
						pressed: true,
						repeat,
						modifiers,
					}],
					..Default::default()
				},
				|ui| messaging.composer(ui, &mut state, channel, &ctx, &mut commands),
			)
			.drop_without_applying_deltas();
			let sends = !busy && allowed && modifiers == egui::Modifiers::NONE && !repeat;
			assert_eq!(commands.len(), usize::from(sends));
			if modifiers == egui::Modifiers::SHIFT {
				assert_eq!(state.drafts[&channel], "\n");
			}
			if sends {
				assert!(
					matches!(&commands[0], Command::Send { content, .. } if content.is_empty())
				);
				assert_eq!(state.pending[0].attachments, ["synthetic.txt"]);
				assert!(messaging.attachment.is_none());
				assert!(messaging.upload_busy);
				let mut release = edit_key(egui::Key::Enter);
				if let egui::Event::Key { pressed, .. } = &mut release {
					*pressed = false;
				}
				assert!(
					edit_frame(
						&ctx,
						&mut messaging,
						&mut state,
						vec![release, edit_key(egui::Key::Enter)],
					)
					.is_empty(),
					"Another Send before desktop dispatch must not enqueue the attachment again"
				);
			} else {
				assert!(
					messaging.attachment.is_some(),
					"Unavailable sends retain selected metadata"
				);
			}
		}
	}

	#[test]
	fn rendering_does_not_hide_saved_drafts_and_clears_survive_hydration() {
		let mut state = State {
			selected: Some(Id(1)),
			channels: vec![model::Channel {
				last_message: None,
				id: Id(1),
				guild: None,
				parent_id: None,
				position: 0,
				name: "Synthetic".into(),
				kind: 1,
				recipients: Vec::new(),
				member_list_id: None,
				tags: None,
				message_count: None,
				icon: None,
			}],
			..Default::default()
		};
		let mut messaging = MessagingUi {
			draft_restore_pending: true,
			..Default::default()
		};
		let mut output = egui::Context::default().run_ui(Default::default(), |ui| {
			messaging.show(ui, &mut state);
		});
		output.textures_delta.clear();
		assert!(
			state.drafts.is_empty(),
			"merely viewing must not suppress asynchronous hydration"
		);
		state
			.drafts
			.entry(Id(1))
			.or_insert("saved synthetic draft".into());
		assert_eq!(state.drafts[&Id(1)], "saved synthetic draft");
		messaging.clear_draft(&mut state, Id(1));
		for _ in 0..3 {
			egui::Context::default()
				.run_ui(Default::default(), |ui| {
					messaging.show(ui, &mut state);
				})
				.drop_without_applying_deltas();
		}
		assert!(
			state.drafts.contains_key(&Id(1)),
			"unchanged renders must retain the explicit clear until hydration completes"
		);
		state
			.drafts
			.entry(Id(1))
			.or_insert("old disk snapshot".into());
		assert!(state.drafts[&Id(1)].is_empty());

		messaging.draft_restore_pending = false;
		state.drafts.retain(|_, text| !text.is_empty());
		for channel in 1..=64 {
			state.drafts.insert(Id(channel), "synthetic".into());
		}
		messaging.clear_draft(&mut state, Id(1));
		assert_eq!(
			state.drafts.len(),
			63,
			"clearing at the slot limit must free capacity"
		);
		assert_eq!(messaging.draft_changes, [Id(1), Id(1)]);
	}
}

/// Offline guild/private-call member-row check; no service or audio access.
#[cfg(debug_assertions)]
pub fn debug_member_voice_status_check(mut state: State, mut private: State) {
	let entry = state.voice.roster[1].clone();
	state.voice.roster = vec![entry.clone()];
	state.voice.active = None;
	let text_channel = state
		.channels
		.iter()
		.find(|channel| channel.guild == Some(entry.guild) && channel.kind == 0)
		.unwrap()
		.id;
	let mut member = entry.member.clone().unwrap();
	member.custom_status = Some("Synthetic status".into());
	state.selected = Some(text_channel);
	state.gateway_connected = true;
	state.members = Some(model::MemberList {
		missing_identity: vec![],
		guild: Some(entry.guild),
		channel: text_channel,
		request: 0,
		start: 0,
		slots: vec![Some(model::MemberSlot::Person(member))],
		total: 1,
		lazy: false,
		freshness: model::Freshness::Fresh,
		groups: vec![],
		ranges: vec![],
	});
	let mut view = MessagingUi {
		language: i18n::Language::English,
		..Default::default()
	};
	let ctx = egui::Context::default();
	let render = |view: &mut MessagingUi, state: &mut State| {
		let output = ctx.run_ui(
			egui::RawInput {
				screen_rect: Some(egui::Rect::from_min_size(
					egui::Pos2::ZERO,
					egui::vec2(240.0, 100.0),
				)),
				..Default::default()
			},
			|ui| view.member_rows(ui, state, &mut vec![]),
		);
		let labels = output
			.shapes
			.iter()
			.filter_map(|shape| match &shape.shape {
				egui::Shape::Text(text) => Some(text.galley.text().to_owned()),
				_ => None,
			})
			.collect::<Vec<_>>();
		output.drop_without_applying_deltas();
		labels
	};
	for _ in 0..2 {
		let labels = render(&mut view, &mut state);
		assert!(labels.iter().any(|text| text == "In voice"));
		assert!(labels.iter().any(|text| text == "Synthetic status"));
	}
	state.gateway_connected = false;
	assert!(
		!render(&mut view, &mut state)
			.iter()
			.any(|text| text == "In voice")
	);
	state.gateway_connected = true;
	state.voice.roster[0].guild = Id(999);
	assert!(
		!render(&mut view, &mut state)
			.iter()
			.any(|text| text == "In voice")
	);
	state.voice.roster[0].guild = entry.guild;
	state.voice.roster.clear();
	assert!(
		!render(&mut view, &mut state)
			.iter()
			.any(|text| text == "In voice")
	);
	state.voice.roster.push(entry.clone());
	state.channels.retain(|channel| channel.id != entry.channel);
	assert!(
		!render(&mut view, &mut state)
			.iter()
			.any(|text| text == "In voice")
	);

	let channel = private.selected.unwrap();
	let user = private.channel(channel).unwrap().recipients[0].clone();
	private.gateway_connected = true;
	private.members = Some(model::MemberList {
		missing_identity: vec![],
		guild: None,
		channel,
		request: 0,
		start: 0,
		slots: vec![Some(model::MemberSlot::Person(model::Member {
			user: user.clone(),
			roles: vec![],
			nick: None,
			status: Some("online".into()),
			custom_status: Some("Synthetic status".into()),
			activities: vec![],
			clients: Default::default(),
		}))],
		total: 1,
		lazy: false,
		freshness: Freshness::Fresh,
		groups: vec![],
		ranges: vec![],
	});
	// The peer is in a shared server, while the viewer is looking at their DM.
	let mut server_entry = entry.clone();
	server_entry.participant.user = user.id;
	private.voice.roster = vec![server_entry];
	assert!(
		render(&mut view, &mut private)
			.iter()
			.any(|text| text == "In voice")
	);
	private.apply(client_core::Envelope {
		generation: private.generation,
		event: client_core::Event::UserAction(client_core::user_actions::Event::Friends(Some(
			vec![(user.clone(), "synthetic".into())],
		))),
	});
	private.direct_presences.push(model::MemberPresence {
		user: user.id,
		status: Some("online".into()),
		custom_status: Some("Synthetic status".into()),
		activities: vec![],
		clients: Default::default(),
	});
	let check_cards = |view: &mut MessagingUi, state: &mut State, header: bool| {
		let check_dm_alignment = |output: &egui::FullOutput| {
			let text_rect = |label: &str| {
				output
					.shapes
					.iter()
					.find_map(|shape| match &shape.shape {
						egui::Shape::Text(text) if text.galley.text() == label => {
							Some(egui::Rect::from_min_size(text.pos, text.galley.size()))
						}
						_ => None,
					})
					.unwrap()
			};
			let name = text_rect(&user.name);
			let status = text_rect("Synthetic status");
			let avatar = text_rect("RS"); // The synthetic peer's avatar initials.
			assert!(status.top() >= name.bottom(), "DM status overlaps its name");
			assert!(
				(name.union(status).center().y - avatar.center().y).abs() < 1.0,
				"DM name/status block is not centered beside its avatar"
			);
		};
		for surface in ["friends", "profile", "dm-list", "dm-header"]
			.into_iter()
			.filter(|surface| header || *surface != "dm-header")
		{
			for frame in 0..2 {
				let output = ctx.run_ui(
					egui::RawInput {
						screen_rect: Some(egui::Rect::from_min_size(
							egui::Pos2::ZERO,
							egui::vec2(800.0, 700.0),
						)),
						..Default::default()
					},
					|ui| match surface {
						"friends" => view.friends_page(ui, state, &mut vec![]),
						"profile" => {
							profiles::show_with_session(
								ui,
								&user,
								None,
								state,
								&mut view.avatars,
								&mut None,
								&mut markdown::FormatCache::default(),
								true,
								egui::pos2(50.0, 50.0),
								&mut profiles::ProfileSession::default(),
							);
						}
						"dm-list" => {
							view.channel_list(ui, state);
						}
						_ => view.channel_header_row(ui, state, false, false, false, &mut vec![]),
					},
				);
				let found = output.shapes.iter().any(|shape| {
					matches!(&shape.shape,
				egui::Shape::Text(text) if text.galley.text() == "In voice")
				});
				if surface == "dm-list" && frame == 1 {
					check_dm_alignment(&output);
					let voice = std::mem::take(&mut state.voice);
					let status_only = ctx.run_ui(egui::RawInput::default(), |ui| {
						view.channel_list(ui, state);
					});
					check_dm_alignment(&status_only);
					status_only.drop_without_applying_deltas();
					state.voice = voice;
				}
				output.drop_without_applying_deltas();
				if frame == 1 {
					assert!(found, "shared-server voice badge missing from {surface}");
				}
			}
		}
	};
	check_cards(&mut view, &mut private, true);
	private.gateway_connected = false;
	assert!(!profiles::voice_users(&private).contains(&user.id));
	assert!(!profiles::user_in_voice(&private, user.id));
	private.gateway_connected = true;
	let guild = private
		.guilds
		.iter()
		.find(|guild| guild.id == entry.guild)
		.unwrap()
		.clone();
	private.guilds.retain(|guild| guild.id != entry.guild);
	assert!(!profiles::voice_users(&private).contains(&user.id));
	assert!(!profiles::user_in_voice(&private, user.id));
	private.guilds.push(guild);
	private.voice.roster.clear();
	assert!(
		!render(&mut view, &mut private)
			.iter()
			.any(|text| text == "In voice")
	);
	private.demo = false;
	private.apply_voice(client_core::voice::Event::Call {
		channel,
		ringing: Some(vec![]),
		unavailable: false,
		participants: Some(vec![client_core::voice::Participant {
			user: user.id,
			muted: false,
			deafened: false,
			server_muted: false,
			server_deafened: false,
			stage: Default::default(),
			video: false,
			streaming: false,
		}]),
	});
	private.demo = true;
	for _ in 0..2 {
		assert!(
			render(&mut view, &mut private)
				.iter()
				.any(|text| text == "In voice")
		);
	}
	private.selected = Some(entry.channel);
	assert!(profiles::voice_users(&private).contains(&user.id));
	assert!(profiles::user_in_voice(&private, user.id));
	check_cards(&mut view, &mut private, false);
	private.selected = Some(channel);
	private.demo = false;
	private.apply_voice(client_core::voice::Event::Call {
		channel,
		ringing: None,
		participants: Some(vec![]),
		unavailable: false,
	});
	private.demo = true;
	assert!(
		!render(&mut view, &mut private)
			.iter()
			.any(|text| text == "In voice")
	);

	println!(
		"Member voice status debug check passed: shared-server voice across member rows, friends, profile, DM list and header; private calls, disconnect and departure. Synthetic egui rendering only."
	);
}

#[cfg(debug_assertions)]
pub fn debug_member_search_check(state: &State, channel: Id) {
	mentions::debug_member_search_check(state, channel);
}

#[cfg(debug_assertions)]
pub fn debug_suggestion_pointer_check(state: &mut State, channel: Id) {
	mentions::debug_pointer_check(state, channel);
}

#[cfg(debug_assertions)]
pub fn debug_role_mentions_check(state: &mut State) {
	mentions::debug_role_mentions_check(state);
}

#[cfg(debug_assertions)]
pub fn debug_forward_check(state: &mut State) {
	let source = state
		.timeline
		.row_ids()
		.find(|id| state.can_forward(*id))
		.expect("forwardable fixture");
	let target = state.selected.unwrap();
	let draft = state.drafts.get(&target).cloned();
	let mut dialog = forwarding::ForwardDialog::default();
	dialog.open(state, source);
	let ctx = egui::Context::default();
	let mut avatars = avatars::Avatars::default();
	for _ in 0..2 {
		ctx.run_ui(egui::RawInput::default(), |ui| {
			dialog.show(ui.ctx(), state, &mut avatars, &mut Vec::new())
		})
		.drop_without_applying_deltas();
	}
	// Exercise the shared row through its text area, not just the checkbox.
	let row_ctx = egui::Context::default();
	let mut selected = false;
	let mut row = egui::Rect::NOTHING;
	let mut draw = |events| {
		row_ctx
			.run_ui(
				egui::RawInput {
					events,
					..Default::default()
				},
				|ui| {
					ui.set_width(320.0);
					let response =
						design::selection_row(ui, selected, "Destination", "Server", |_, _| {});
					row = response.rect;
					assert_eq!(row.height(), 56.0);
					if response.clicked() {
						selected = !selected;
					}
					icons::button(
						ui,
						icons::Icon::Forward,
						28.0,
						&crate::i18n::translate("lib-debug-forward-check-forward-message"),
					);
				},
			)
			.drop_without_applying_deltas();
	};
	draw(Vec::new());
	draw(Vec::new());
	let point = egui::pos2(90.0, 22.0);
	for pressed in [true, false] {
		draw(vec![
			egui::Event::PointerMoved(point),
			egui::Event::PointerButton {
				pos: point,
				button: egui::PointerButton::Primary,
				pressed,
				modifiers: egui::Modifiers::NONE,
			},
		]);
	}
	assert!(selected, "Clicking the row label selects the destination");
	assert!(state.prepare_forward(source, &[target; 6], "").is_empty());
	assert!(
		state
			.prepare_forward(source, &[target, target], "")
			.is_empty()
	);
	let commands = state.prepare_forward(source, &[target], "Optional note");
	assert_eq!(commands.len(), 2);
	assert!(
		matches!(&commands[0], Command::Forward { message, channel, .. } if *message == source && *channel == target)
	);
	assert!(
		matches!(&commands[1], Command::Send { content, reply: None, .. } if content == "Optional note")
	);
	assert_eq!(state.drafts.get(&target).cloned(), draft);
	for command in commands {
		state.command_rejected(command);
	}
	assert!(
		state
			.pending
			.iter()
			.all(|p| p.delivery != model::Delivery::Sending)
	);
}

/// Offline issue #559 check; no transport, image worker or audio device is attached.
#[cfg(feature = "demo")]
pub fn debug_gif_favorites_check(mut message: model::Message) -> Vec<model::Gif> {
	let proxy = "https://images-ext-1.discordapp.net/external/synthetic/https/example.org/wave.gif";
	let cases = [
		(
			"https://cdn.discordapp.com/attachments/1/2/wave.gif?ex=abc&is=def&hm=123",
			None,
			false,
		),
		("https://example.org/wave.gif", Some(proxy), false),
		("https://media.tenor.com/synthetic/wave.mp4", None, true),
	];
	let mut gifs = Vec::new();
	for (url, proxy_url, animated) in cases {
		let media = model::EmbedMedia {
			url: Some(url.into()),
			proxy_url: proxy_url.map(str::to_owned),
			width: 320,
			height: 200,
			..Default::default()
		};
		let gif = embeds::gif_for_media(&media, None, animated).expect("GIF can be starred");
		assert_eq!(gif.url, url);
		assert_eq!(gif.preview, proxy_url.unwrap_or(url));
		gifs.push(gif);
	}
	assert!(
		embeds::gif_for_media(
			&model::EmbedMedia {
				url: Some("https://example.org/still.png".into()),
				proxy_url: Some(proxy.into()),
				..Default::default()
			},
			None,
			false
		)
		.is_some()
	); // Proxy itself identifies an animated image.
	assert!(
		embeds::gif_for_media(
			&model::EmbedMedia {
				url: Some("https://example.org/wave.gif".into()),
				..Default::default()
			},
			None,
			false
		)
		.is_none(),
		"foreign originals do not authorize a fetch"
	);
	for bad in [
		"https://user@example.org/x.gif",
		"http://example.org/x.gif",
		"https://example.org/x.gif\n",
	] {
		assert!(!model::valid_gif_favorite_url(bad));
	}
	let ctx = egui::Context::default();
	let mut state = client_core::State::default();
	let mut images = avatars::Avatars::default();
	let mut download = DownloadUi::default();
	message.attachments.clear();
	message.attachments.push(model::Attachment {
		id: model::Id(2),
		filename: "wave.gif".into(),
		description: None,
		content_type: Some("image/gif".into()),
		size: 100,
		spoiler: false,
		duration_ms: None,
		waveform: vec![],
		media: model::EmbedMedia {
			url: Some(gifs[0].url.clone()),
			width: 320,
			height: 200,
			..Default::default()
		},
	});
	let mut star = None;
	for click in [false, true] {
		let events = star
			.filter(|_| click)
			.map(|pos| {
				vec![
					egui::Event::PointerMoved(pos),
					egui::Event::PointerButton {
						pos,
						button: egui::PointerButton::Primary,
						pressed: true,
						modifiers: Default::default(),
					},
					egui::Event::PointerButton {
						pos,
						button: egui::PointerButton::Primary,
						pressed: false,
						modifiers: Default::default(),
					},
				]
			})
			.unwrap_or_default();
		let output = ctx.run_ui(
			egui::RawInput {
				screen_rect: Some(egui::Rect::from_min_size(
					egui::Pos2::ZERO,
					egui::vec2(640.0, 480.0),
				)),
				events,
				..Default::default()
			},
			|ui| {
				attachments::show(
					ui,
					&message,
					&mut images,
					&mut None,
					&mut None,
					&mut download,
					&mut audio::AudioUi::default(),
					&mut video::VideoUi::default(),
					true,
					&mut select::Surface::new(ui, "gif-check"),
					design::MessageCardSurface::Opaque,
				);
			},
		);
		star = output.shapes.iter().find_map(|shape| match &shape.shape {
			egui::Shape::Rect(rect) if rect.rect.size() == egui::Vec2::splat(30.0) => {
				Some(rect.rect.center())
			}
			_ => None,
		});
		output.drop_without_applying_deltas();
	}
	let clicked = download
		.gif_favorite_request
		.take()
		.expect("attachment star click");
	assert!(state.toggle_gif_favorite(&clicked));
	assert!(state.is_gif_favorite(&clicked));
	assert!(state.toggle_gif_favorite(&clicked));
	assert!(!state.is_gif_favorite(&clicked));
	gifs
}

/// Offline exercise of the same picker, composer, Markdown and optimistic row paths.
#[cfg(all(debug_assertions, feature = "demo"))]
pub fn debug_image_sharing(state: &mut State) -> (Vec<model::ImageShare>, String) {
	let ctx = egui::Context::default();
	let channel = state.selected.unwrap();
	let mut view = MessagingUi::default();
	let mut commands = Vec::new();
	let picked = mentions::debug_image_completion(state);
	assert_eq!(picked, emoji_picker::debug_fallback_choice(state));
	let sticker = model::ImageShare::Sticker {
		id: Id(9201),
		format_type: 1,
	}
	.markdown("Wave [hello]")
	.unwrap();
	let mixed = format!("Hello {picked} {sticker}!");
	assert!(model::ImageShare::markdown_only(&mixed).is_none());
	assert!(model::ImageShare::markdown_only("[evil](https://example.com/image.png)").is_none());
	assert!(
		model::ImageShare::from_url("https://cdn.discordapp.com/emojis/9002.gif?size=64&extra=1")
			.is_none()
	);
	assert!(
		model::ImageShare::from_url("https://cdn.discordapp.com/emojis/09002.gif?size=64")
			.is_none()
	);
	state.drafts.insert(channel, mixed.clone());
	for dark in [true, false] {
		ctx.set_visuals(if dark {
			egui::Visuals::dark()
		} else {
			egui::Visuals::light()
		});
		for _ in 0..2 {
			let output = ctx.run_ui(Default::default(), |ui| {
				view.composer(ui, state, channel, &ctx, &mut commands);
				markdown::Formatted::parse(&mixed).show_with_images(
					ui,
					&mut None,
					&[],
					None,
					&mut profiles::ProfileSession::default(),
					(&mut view.avatars, true, &[]),
					design::MessageCardSurface::Conversation,
				);
			});
			assert!(
				output.shapes.iter().any(
					|shape| matches!(&shape.shape, egui::Shape::Rect(rect) if rect.brush.is_some())
				),
				"native inline artwork renders"
			);
			output.drop_without_applying_deltas();
		}
	}
	assert!(
		commands.is_empty() && view.image_share_requested.is_none(),
		"picking and rendering never sends"
	);
	view.submit_composer(state, &ctx, &mut commands);
	assert!(matches!(commands.as_slice(), [Command::Send { content, .. }] if content == &mixed));
	assert!(state.pending.last().unwrap().attachments.is_empty());
	state.pending.clear();
	commands.clear();
	view.preview_attachment(
		"emoji-9002.gif",
		12,
		Some(egui::ColorImage::filled([4, 4], egui::Color32::WHITE)),
	);
	let command = state.prepare_image_send(&["emoji-9002.gif"]).unwrap();
	view.stage_pending_upload(&ctx, &command);
	let output = ctx.run_ui(Default::default(), |ui| {
		pending::show(
			ui,
			state.pending.last().unwrap(),
			(false, 10, false),
			state,
			(
				&mut view.avatars,
				&mut None,
				&mut view.profile,
				&mut None,
				&mut markdown::FormatCache::default(),
			),
			view.pending_upload.as_ref(),
			(&mut None, &mut None, &mut false),
		);
	});
	assert!(!output.shapes.iter().any(|shape| matches!(&shape.shape, egui::Shape::Text(text) if text.galley.job.text == "Restore to composer")), "sending artwork has no restore button");
	output.drop_without_applying_deltas();
	state.pending.clear();
	view = MessagingUi::default();
	let alone = format!("{picked} {sticker}");
	state.drafts.insert(channel, alone.clone());
	view.submit_composer(state, &ctx, &mut commands);
	view.submit_composer(state, &ctx, &mut commands);
	assert!(commands.is_empty() && state.pending.is_empty());
	let (generation, queued_channel, assets, draft) = view.image_share_requested.take().unwrap();
	assert_eq!((generation, queued_channel), (state.generation, channel));
	assert_eq!(assets.len(), 2);
	assert_eq!(
		state.drafts[&channel], alone,
		"preparation retains the draft until success"
	);
	(assets, draft)
}

/// Offline interaction check for PR 565; never attaches service or image workers.
#[cfg(all(debug_assertions, feature = "demo"))]
pub fn debug_pr565(mut state: State, mut user: model::User) {
	state.debug_profile_note_read_check(&user);
	profiles::debug_activity_panel(&state);
	profiles::debug_board_layout();
	user.avatar = Some("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into());
	markdown::Formatted::debug_quote_regressions();
	let ctx = egui::Context::default();
	ctx.enable_accesskit();
	design::apply(&ctx);
	let mut view = MessagingUi::default();
	view.preview_profile(user.clone());
	let frame = |view: &mut MessagingUi, state: &mut State, events| {
		let mut output = ctx.run_ui(
			egui::RawInput {
				screen_rect: Some(egui::Rect::from_min_size(
					egui::Pos2::ZERO,
					egui::vec2(1120.0, 900.0),
				)),
				events,
				focused: true,
				..Default::default()
			},
			|ui| {
				view.show(ui, state);
			},
		);
		output.textures_delta.clear();
		output
	};
	let avatar = |output: &egui::FullOutput, label: &str| {
		let bounds = output
			.platform_output
			.accesskit_update
			.as_ref()
			.unwrap()
			.nodes
			.iter()
			.find_map(|(_, node)| {
				(node.label() == Some(label))
					.then(|| node.bounds())
					.flatten()
					.filter(|rect| rect.x1 - rect.x0 > 50.0)
			})
			.expect("profile avatar is accessible");
		egui::pos2(
			((bounds.x0 + bounds.x1) / 2.0) as f32,
			((bounds.y0 + bounds.y1) / 2.0) as f32,
		)
	};
	let click = |view: &mut MessagingUi, state: &mut State, pos| {
		for pressed in [true, false] {
			frame(
				view,
				state,
				vec![
					egui::Event::PointerMoved(pos),
					egui::Event::PointerButton {
						pos,
						button: egui::PointerButton::Primary,
						pressed,
						modifiers: egui::Modifiers::NONE,
					},
				],
			);
		}
	};
	for _ in 0..3 {
		frame(&mut view, &mut state, vec![]);
	}
	let compact = frame(&mut view, &mut state, vec![]);
	let connection = state
		.profile
		.as_ref()
		.unwrap()
		.data
		.as_ref()
		.unwrap()
		.connections[0]
		.name
		.clone();
	assert!(!compact.shapes.iter().any(
		|shape| matches!(&shape.shape, egui::Shape::Text(text) if text.galley.text() == connection)
	));
	let own = state.user.clone().unwrap();
	state.own_profile.data = Some(profiles::synthetic(&own, None));
	assert_eq!(
		profiles::known_username(&state, own.id).as_deref(),
		Some("serein.preview")
	);
	click(
		&mut view,
		&mut state,
		avatar(&compact, &i18n::translate("profiles-view-full-profile")),
	);
	for _ in 0..3 {
		frame(&mut view, &mut state, vec![]);
	}
	let full = frame(&mut view, &mut state, vec![]);
	assert!(
		ctx.memory(|memory| memory.area_rect(egui::Id::unique("user-profile-full")))
			.is_some()
	);
	assert!(
		full.shapes.iter().any(
			|shape| matches!(&shape.shape, egui::Shape::Text(text) if text.galley.text() == connection)
		),
		"full profile retains connections"
	);
	click(
		&mut view,
		&mut state,
		avatar(
			&full,
			&i18n::translate("profiles-show-view-profile-picture"),
		),
	);
	let image = &view
		.profile_image
		.as_ref()
		.expect("full avatar opens image")
		.1;
	assert!(image.media.url.as_ref().unwrap().contains("size=2048"));
	view.profile_image = None;
	user.webhook = true;
	view.preview_profile(user);
	for _ in 0..3 {
		frame(&mut view, &mut state, vec![]);
	}
	let webhook = frame(&mut view, &mut state, vec![]);
	click(
		&mut view,
		&mut state,
		avatar(
			&webhook,
			&i18n::translate("profiles-show-view-profile-picture"),
		),
	);
	assert!(
		view.profile_image.is_some(),
		"webhook avatar still opens its image"
	);
	emoji_picker::debug_gif_retry(&mut state);
	println!(
		"Profile avatar navigation, connections, username fallback, GIF retry and Markdown checks passed."
	);
}

#[cfg(all(debug_assertions, feature = "demo"))]
pub use timeline::debug_copy_feedback_check;
