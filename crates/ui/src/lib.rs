#![allow(clippy::items_after_test_module)]
//! Native egui views; emits commands without owning transports or session credentials.
mod account_badge;
mod account_menu;
mod archives;
mod audio;
mod forwarding;
pub use audio::{AudioCommand, AudioState, AudioUi};
mod video;
pub use video::{VideoCommand, VideoState, VideoUi};
mod attachments;
pub use attachments::DownloadUi;
mod avatars;
pub use avatars::{EMBED_EDGE, GifFrames, LARGE_EDGE, fit_edge};
mod categories;
#[cfg(feature = "demo")]
pub use categories::debug_thread_navigation_check;
mod channel_marks;
mod channel_menu;
mod channel_permissions;
#[cfg(test)]
mod channel_welcome_tests;
mod components;
mod composer_text;
pub mod design;
mod embeds;
mod extensions_ui;
mod theme_editor;
mod thread_create;
pub use extensions_ui::{ExtensionContext, ExtensionEntry, ExtensionRequest, ExtensionUi};
pub mod emoji;
mod emoji_details;
mod emoji_picker;
pub mod fonts;
mod formatting;
mod forum;
mod friends;
mod group_menu;
mod guild_folders;
mod highlight;
pub mod icons;
mod invites;
mod local_time;
mod markdown;
mod member_search;
mod mentions;
mod messaging_permissions;
mod notification_settings;
mod notifications;
mod pending;
mod post_menu;
mod profiles;
mod slash_builtin;
mod slash_commands;
mod stickers;
/// Synthetic global profile used exclusively by the desktop's offline command adapter.
#[cfg(any(test, feature = "demo"))]
pub fn synthetic_own_profile(user: &model::User) -> model::UserProfile {
	profiles::synthetic(user, None)
}
mod contact_editor;
pub mod dialog;
mod join_server;
mod keybinds;
mod profile_edit;
mod reactions;
mod reading;
pub mod screen;
pub mod scroll;
mod search;
pub mod select;
mod server_admin;
mod server_audit_log;
mod server_integrations;
mod server_invite;
mod server_invites;
mod server_menu;
mod server_roles;
mod server_settings;
mod settings;
mod shortcuts;
mod switcher;
mod thumbhash;
mod timeline;
#[cfg(test)]
mod title_bar_tests;
pub mod toasts;
mod typing;
pub mod updates;
mod user_menu;
mod verification;
mod voice;
use client_core::{Command, MAX_CONTENT, MAX_DRAFT_BYTES, NavStep, State};
use egui::{RichText, TextEdit};
use model::{Freshness, Id};
pub use verification::VerificationUi;
pub use voice::StageFocus;

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

pub struct AttachmentPaste {
	pub target: egui::Id,
	pub text: Option<String>,
	pub image: Option<std::sync::Arc<egui::ColorImage>>,
}

#[derive(Default)]
pub struct MessagingUi {
	forwarding: forwarding::ForwardDialog,
	pub image_sharing_enabled: bool,
	pub image_share_requested: Option<model::ImageShare>,
	pub interaction_file_request: Option<String>,
	interaction_components: components::Components,
	pub verification: VerificationUi,
	pub extensions: ExtensionUi,
	friends: friends::Friends,
	account_menu: account_menu::AccountMenu,
	pub own_presence: model::OwnPresence,
	/// When the custom status clears itself, in milliseconds since the Unix epoch. The
	/// deadline is local: Discord carries it in account settings, which this client does
	/// not write, so the host enforces it and republishes the cleared presence.
	pub own_presence_expires: Option<u64>,
	pub own_presence_changed: bool,
	pub own_presence_status: &'static str,
	group_menu: group_menu::GroupMenu,
	server_menu: server_menu::ServerMenu,
	channel_menu: channel_menu::ChannelMenu,
	pub channel_preferences: model::ChannelPreferences,
	pub channel_preferences_changed: bool,
	pub channel_preferences_loaded: bool,
	pub channel_preferences_load_pending: bool,
	pub channel_preferences_reload: bool,
	pub channel_preferences_save_pending: bool,
	pub channel_preferences_status: &'static str,
	join_server: join_server::JoinDialog,
	folder_ui: guild_folders::FolderUi,
	rail_cache: notifications::RailCache,
	composer_layout: composer_text::Layout,
	channel_cache: categories::Cache,
	channel_move: Option<(Id, client_core::channel_actions::Action)>,
	search: search::SearchUi,
	settings: settings::Settings,
	server_settings: server_settings::Editor,
	server_icon_sequence: u64,
	server_emoji_sequence: u64,
	server_role_icon_sequence: u64,
	switcher: switcher::Switcher,
	focus_switched_composer: bool,
	#[cfg(feature = "demo")]
	preview_slash_commands: bool,
	switcher_frame: bool,
	archives: archives::ArchivesUi,
	archive_parent: Option<Id>,
	thread_create: thread_create::ThreadCreateUi,
	forum: forum::ForumUi,
	scroll: scroll::Session,
	side_press: scroll::SidePress,
	timeline: timeline::TimelineView,
	edit_modified: Option<(Id, Id, bool)>,
	edit_undo_cleared: bool,
	edit_widget_id: Option<egui::Id>,
	edit_closed_channel: Option<Id>,
	avatars: avatars::Avatars,
	profile: Option<model::User>,
	profile_image: Option<(u64, model::Attachment)>,
	user_action: Option<user_menu::Action>,
	pending_mention: Option<Id>,
	contact_editor: contact_editor::ContactEditor,
	profile_link: Option<String>,
	profile_formatted: markdown::FormatCache,
	pub reading_preferences: model::ReadingPreferences,
	pub show_hidden_channels: bool,
	pub hide_title_bar: bool,
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
	reading_sidebar_applied: Option<u16>,
	reading_sidebar_constrained: bool,
	reading_zoom_pending: bool,
	/// Slider value while the pointer is still down; zoom is applied on release so the
	/// slider does not rescale under the cursor mid-drag.
	reading_zoom_draft: Option<u16>,
	/// Where the open profile was requested from; the popout is placed beside it.
	profile_anchor: Option<(Id, egui::Pos2)>,
	profile_trigger: Option<egui::Rect>,
	friend_removal: Option<(u64, model::User)>,
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
	pub draft_changes: Vec<Id>,
	pub draft_restore_pending: bool,
	pub attachment: Option<(String, u64)>,
	pub attachment_files: Vec<(String, u64)>,
	pub remove_attachment_index: Option<usize>,
	/// Downscaled pixels per selected file (`None` for non-images), produced off the render thread.
	pub attachment_previews: Vec<Option<std::sync::Arc<egui::ColorImage>>>,
	/// GPU copies of `attachment_previews`, keyed by the pixel buffer they were uploaded from.
	attachment_textures: Vec<Option<(usize, egui::TextureHandle)>>,
	pub attach_requested: bool,
	pub attachment_paste_requested: Option<AttachmentPaste>,
	pub pasted_text: Option<(Id, egui::Id, String)>,
	paste_key_handled: bool,
	pasted_text_frame: Option<u64>,
	pub remove_attachment_requested: bool,
	pub cancel_upload_requested: bool,
	pub upload_busy: bool,
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
	/// Decoded remote cameras by user; the desktop bounds and replaces them.
	pub voice_remote_video: Vec<(Id, egui::TextureHandle)>,
	/// Latest picture of the screen share this device chose to watch.
	pub voice_stream_view: Option<egui::TextureHandle>,
	pub voice_stream_status: &'static str,
	/// Session-only stream playback level; unset is 100%. Mute preserves the level.
	voice_stream_volume: Option<u16>,
	voice_stream_muted: bool,
	/// Enlarged stage tile; cleared when it stops showing video or on Escape.
	pub voice_focus: Option<voice::StageFocus>,
	/// Whether the other participants stay visible as a strip under the enlarged tile.
	pub voice_focus_participants: bool,
	/// Session-only visibility of the selected guild voice channel's chat.
	pub voice_chat_open: bool,
	/// Tile click to start (`Some(user)`) or stop (`None`) watching, applied by the stage.
	watch_request: Option<Option<Id>>,
	pub voice_inputs: Vec<(String, String)>,
	pub voice_outputs: Vec<(String, String)>,
	pub voice_input: Option<String>,
	pub voice_output: Option<String>,
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
	pub voice_preview_requested: bool,
	pub voice_preview_level: Option<f32>,
	pub voice_preview_status: &'static str,
	/// Device-local application shortcuts; the desktop host mirrors the global binding.
	pub keybinds: model::Keybinds,
	keybind_capture: Option<model::KeybindAction>,
	pub global_keybind_status: &'static str,
	/// Server folders the owner left open; restored from device preferences at startup.
	pub expanded_folders: Vec<u64>,
	pub voice_ptt_active: bool,
	pub voice_privacy_code: Option<String>,
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
	edit_sent: bool,
	deleting: Option<(Id, Id)>,
	ime_active: bool,
	mention_menu: mentions::Menu,
	slash_commands: slash_commands::Menu,
	slash_direct: Option<slash_builtin::PendingMessage>,
	mention_lookup: member_search::Search,
	emoji_picker: emoji_picker::Picker,
	reaction_picker: emoji_picker::Picker,
	/// Set when the user picks a theme preset; the host persists it.
	pub theme_variant_changed: Option<design::Variant>,
	pub primary_color: Option<[u8; 3]>,
	pub transparency_blur: bool,
	pub transparency: u8,
	pub blur: u8,
	pub transparent_all: bool,
}

/// Context strip (reply/edit) drawn as the rounded top of the composer block.
fn composer_cap(
	ui: &mut egui::Ui,
	colors: &design::Palette,
	add_contents: impl FnOnce(&mut egui::Ui),
) -> egui::Rect {
	egui::Frame::new()
		.fill(design::mix(colors.raised, colors.base, 0.45))
		.corner_radius(egui::CornerRadius {
			nw: 8,
			ne: 8,
			sw: 0,
			se: 0,
		})
		.inner_margin(egui::Margin {
			left: 16,
			right: 8,
			top: 5,
			bottom: 5,
		})
		.show(ui, |ui| {
			ui.set_min_width((ui.available_width()).max(0.0));
			ui.horizontal(|ui| add_contents(ui));
		})
		.response
		.rect
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
			"Ping the original author",
		)
	});
	response.on_hover_text(hover);
}

enum MentionWrite {
	Inserted,
	DidNotFit,
}

impl MessagingUi {
	pub fn interaction_modal_id(&self) -> Option<Id> {
		self.interaction_components.modal_id()
	}
	pub fn set_interaction_files(&mut self, custom_id: &str, files: Vec<(usize, String)>) {
		self.interaction_components.set_files(custom_id, files);
	}

	/// Feed the frame's middle button before `show`. Never fed means never pressed.
	pub fn middle_button(&mut self, middle: scroll::Middle) {
		self.scroll.middle(middle);
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
		ctx.input(|input| {
			input.focused
				&& !ctx.egui_wants_keyboard_input()
				&& crate::keybinds::down(
					input,
					self.keybinds.chord(model::KeybindAction::PushToTalk),
				)
		})
	}

	/// Returns focused-window mute/deafen presses that were not claimed by a native global hotkey.
	pub fn voice_toggle_pressed(&self, ctx: &egui::Context, global_mask: u8) -> u8 {
		if !ctx.input(|input| input.focused) || ctx.egui_wants_keyboard_input() {
			return 0;
		}
		ctx.input_mut(|input| {
			let mut toggles = 0;
			if global_mask & 1 == 0
				&& crate::keybinds::pressed(
					input,
					self.keybinds.chord(model::KeybindAction::ToggleMute),
				) {
				toggles |= 1;
			}
			if global_mask & 2 == 0
				&& crate::keybinds::pressed(
					input,
					self.keybinds.chord(model::KeybindAction::ToggleDeafen),
				) {
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
	/// Fixture-only: open the Threads dialog for `parent` at startup, as the header control would.
	#[cfg(any(test, feature = "demo"))]
	pub fn preview_threads(&mut self, parent: Id) {
		self.archive_parent = Some(parent);
	}
	/// Fixture-only entry point: opens People and the profile card for `user` as if clicked.
	#[cfg(any(test, feature = "demo"))]
	pub fn preview_profile(&mut self, user: model::User) {
		self.members_narrow_open = true;
		self.profile = Some(user);
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
						title: Some("Serein community".into()),
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
	pub fn clear_avatars(&mut self) {
		self.avatars = avatars::Avatars::default();
	}
	pub fn take_avatar_requests(&mut self) -> Vec<String> {
		self.avatars.take_requests()
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
		// Window preferences belong to the application, not the account being cleared.
		*self = Self {
			build: self.build,
			// The switcher roster belongs to the device, not to the account being cleared.
			accounts: std::mem::take(&mut self.accounts),
			updates: std::mem::take(&mut self.updates),
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
	}
	pub fn restore_channel_preferences(&mut self, preferences: model::ChannelPreferences) {
		if !self.channel_preferences_changed && preferences.is_valid() {
			self.channel_preferences = preferences;
			self.channel_preferences_loaded = true;
			self.channel_preferences_status = "";
			self.channel_cache.invalidate();
		}
	}
	pub fn has_edit(&self) -> bool {
		self.editing.is_some()
	}
	pub fn has_edit_in(&self, channel: Option<Id>) -> bool {
		self.editing
			.as_ref()
			.is_some_and(|(edit_channel, _, _)| Some(*edit_channel) == channel)
	}
	pub fn messages_deleted(&mut self, ctx: &egui::Context, channel: Id, ids: &[Id]) {
		if ids.len() > 100 {
			return;
		}
		if self
			.deleting
			.is_some_and(|(c, id)| c == channel && ids.contains(&id))
		{
			self.deleting = None;
		}
		let Some((edit_channel, message, _)) = &self.editing else {
			return;
		};
		if *edit_channel != channel || !ids.contains(message) {
			return;
		}
		let untouched = self
			.edit_modified
			.is_some_and(|(c, id, modified)| c == channel && id == *message && !modified);
		if let Some(id) = self.edit_widget_id {
			if untouched {
				egui::text_edit::TextEditState::default().store(ctx, id);
			} else if let Some(mut editor) = egui::text_edit::TextEditState::load(ctx, id) {
				editor.clear_undoer();
				editor.store(ctx, id);
			}
		}
		self.edit_sent = false;
		self.edit_undo_cleared = true;
		if untouched {
			self.editing = None;
			self.edit_modified = None;
			self.edit_widget_id = None;
			self.edit_closed_channel = Some(channel);
		}
	}
	fn reconcile_edit(&mut self, state: &State) {
		let Some((channel, id, content)) = &self.editing else {
			self.edit_modified = None;
			self.edit_undo_cleared = false;
			return;
		};
		if self
			.edit_modified
			.is_none_or(|(c, message, _)| c != *channel || message != *id)
		{
			self.edit_undo_cleared = false;
			let modified = state
				.timeline
				.get(*id)
				.filter(|message| message.channel == *channel)
				.is_none_or(|message| message.content != *content);
			self.edit_modified = Some((*channel, *id, modified));
		}
		if state.selected == Some(*channel)
			&& state.timeline.get(*id).is_none()
			&& self.edit_modified.is_some_and(|(_, _, modified)| !modified)
		{
			self.editing = None;
			self.edit_modified = None;
			self.edit_sent = false;
		}
	}
	/// Window title strip: traffic-light inset, centred context title and session state.
	fn title_bar(&mut self, ui: &mut egui::Ui, state: &State, title: &str) {
		let colors = design::palette(ui);
		egui::Panel::top("title-bar")
			.exact_size(36.0)
			.show_separator_line(false)
			.frame(egui::Frame::new().fill(design::section_surface(
				ui,
				design::window_palette(ui).base,
				design::ImageSection::TopBar,
			)))
			.show(ui, |ui| {
				// Caption text belongs to the window drag region, not text selection.
				ui.style_mut().interaction.selectable_labels = false;
				let rect = ui.max_rect();
				design::window_drag(ui, rect);
				let title_rect = egui::Rect::from_center_size(
					rect.center(),
					egui::vec2(rect.width() * 0.3, rect.height()),
				);
				ui.scope_builder(
					egui::UiBuilder::new().max_rect(title_rect).layout(
						egui::Layout::centered_and_justified(egui::Direction::LeftToRight),
					),
					|ui| {
						ui.add(
							egui::Label::new(design::semibold(ui, title, 13.0).color(colors.text))
								.truncate(),
						);
					},
				);

				ui.scope_builder(
					egui::UiBuilder::new()
						.max_rect(egui::Rect::from_min_max(
							egui::pos2(title_rect.right() + 10.0, rect.top()),
							rect.right_bottom() - egui::vec2(12.0, 0.0),
						))
						.layout(egui::Layout::right_to_left(egui::Align::Center)),
					|ui| {
						design::window_controls(ui);
						ui.spacing_mut().item_spacing.x = 10.0;
						if self.updates.available || self.updates.ready {
							let (label, icon) = if self.updates.ready {
								("Restart to update", icons::Icon::Reload)
							} else if self.updates.busy {
								("Updating…", icons::Icon::Download)
							} else {
								("Update available", icons::Icon::Download)
							};
							let font = egui::FontId::new(12.0, design::medium_family(ui.ctx()));
							let galley =
								ui.painter()
									.layout_no_wrap(label.to_owned(), font, colors.accent);
							let icon_size = 13.0;
							let gap = 5.0;
							let pad = egui::vec2(6.0, 2.0);
							let size = egui::vec2(
								galley.size().x + icon_size + gap + pad.x * 2.0,
								galley.size().y.max(icon_size) + pad.y * 2.0,
							);
							let (rect, response) =
								ui.allocate_exact_size(size, egui::Sense::click());
							ui.painter().rect_filled(
								rect,
								255,
								colors.accent.gamma_multiply(if response.hovered() {
									0.24
								} else {
									0.16
								}),
							);
							let icon_rect = egui::Rect::from_center_size(
								egui::pos2(rect.left() + pad.x + icon_size / 2.0, rect.center().y),
								egui::Vec2::splat(icon_size),
							);
							icons::paint(ui.painter(), icon, icon_rect, colors.accent);
							ui.painter().galley(
								egui::pos2(
									icon_rect.right() + gap,
									rect.center().y - galley.size().y / 2.0,
								),
								galley,
								colors.accent,
							);
							if response.on_hover_text(&self.updates.status).clicked() {
								self.open_update_settings();
							}
						} else {
							design::build_badge(ui, self.build);
						}
						if state.demo && !self.updates.available && !self.updates.ready {
							egui::Frame::new()
								.stroke(egui::Stroke::new(1.0, colors.border))
								.corner_radius(10)
								.inner_margin(egui::Margin::symmetric(8, 3))
								.show(ui, |ui| {
									ui.label(
										design::semibold(ui, "OFFLINE PREVIEW", 10.0)
											.color(colors.muted),
									);
								})
								.response
								.on_hover_text("Synthetic data · no network or local storage");
						}
						if !state.demo
							&& state.auth != client_core::auth::AuthState::Authenticated
							&& ui.small_button("Sign in again").clicked()
						{
							self.reconnect_requested = true;
						}
					},
				);
			});
	}
	fn member_rows(&mut self, ui: &mut egui::Ui, state: &mut State, commands: &mut Vec<Command>) {
		let colors = design::palette(ui);
		let cached = state.members_cached();
		let Some(list) = state
			.members
			.as_ref()
			.filter(|list| Some(list.channel) == state.selected)
		else {
			ui.add_space(8.0);
			ui.label(RichText::new("Choose a conversation to see its people.").color(colors.muted));
			return;
		};
		let has_entry = list.slots.iter().any(|slot| slot.is_some()) || cached;
		if !has_entry {
			ui.add_space(8.0);
			if list.freshness != Freshness::Fresh {
				let text = if list.freshness == Freshness::Unavailable {
					"People aren't available in this conversation."
				} else {
					"Loading people…"
				};
				ui.label(RichText::new(text).small().color(colors.muted));
			} else {
				ui.label(
					RichText::new("No people returned for this view.")
						.small()
						.color(colors.muted),
				);
			}
		}
		let channel = list.channel;
		let lazy = list.lazy;
		let start = list.start;
		let guild = list.guild;
		let member_key = (state.generation, channel, list.request);
		let reset_scroll = self.member_extent.is_none_or(|(key, _)| key != member_key);
		if reset_scroll {
			self.member_extent = Some((member_key, 100));
		}
		let total = list.total.min(250_000) as usize;
		let row_count = if lazy {
			self.member_extent.unwrap().1.min(total)
		} else {
			list.slots.len()
		};
		let row_spacing = ui.spacing().item_spacing.y;
		ui.spacing_mut().item_spacing.y = 0.0;
		let mut visible = 0usize..0;
		let mut area = egui::ScrollArea::vertical().auto_shrink([false, false]);
		if reset_scroll {
			area = area.vertical_scroll_offset(0.0);
		}
		let output = self.scroll.attach(ui, ("people", channel), area).show_rows(
			ui,
			42.0,
			row_count,
			|ui, range| {
				visible = range.clone();
				for index in range {
					let slot = if lazy {
						state.member_slot(index).or_else(|| {
							if index >= start {
								list.slots.get(index - start).and_then(|slot| slot.as_ref())
							} else {
								None
							}
						})
					} else {
						list.slots.get(index).and_then(|s| s.as_ref())
					};
					match slot {
						Some(model::MemberSlot::Group(id)) => {
							let name = match id.as_str() {
								"online" => "Online".to_owned(),
								"offline" => "Offline".to_owned(),
								_ => {
									let role = id.parse::<u64>().ok().and_then(|role_id| {
										guild.and_then(|guild| {
											state.guild_roles(guild).and_then(|roles| {
												roles.iter().find(|role| role.id == Id(role_id))
											})
										})
									});
									match role {
										Some(role) if !role.name.is_empty() => role.name.clone(),
										_ => "Role".to_owned(),
									}
								}
							};
							let text = list
								.groups
								.iter()
								.find(|(group_id, _)| group_id == id)
								.map(|(_, count)| format!("{name} - {count}"))
								.unwrap_or(name);
							let (rect, _) = ui.allocate_exact_size(
								egui::vec2(ui.available_width(), 42.0),
								egui::Sense::hover(),
							);
							let mut header = ui.new_child(
								egui::UiBuilder::new()
									.max_rect(egui::Rect::from_min_max(
										rect.left_top() + egui::vec2(8.0, 16.0),
										rect.right_bottom() - egui::vec2(8.0, 0.0),
									))
									.layout(egui::Layout::left_to_right(egui::Align::Center)),
							);
							header
								.add(
									egui::Label::new(
										design::medium(ui, &text, 12.0).color(colors.muted),
									)
									.truncate(),
								)
								.on_hover_text(&text);
						}
						Some(model::MemberSlot::Person(member)) => {
							let name = member
								.nick
								.as_deref()
								.filter(|nick| !nick.is_empty())
								.unwrap_or_else(|| state.user_display_name(&member.user));
							let (status, custom, activities) =
								profiles::member_presence(state, member, guild);
							let subtitle = profiles::subtitle(custom, activities);
							let online =
								status.is_some_and(|s| matches!(s, "online" | "idle" | "dnd"));
							let (rect, response) = ui.allocate_exact_size(
								egui::vec2(ui.available_width(), 42.0),
								egui::Sense::click(),
							);
							response.widget_info(|| {
								egui::WidgetInfo::labeled(
									egui::Role::Button,
									true,
									format!(
										"{name}, {}, {}",
										status.map_or("presence unknown", profiles::presence_label),
										subtitle.as_deref().unwrap_or_default()
									),
								)
							});
							let row = rect.shrink2(egui::vec2(0.0, 1.0));
							if response.hovered() || response.has_focus() {
								ui.painter().rect_filled(
									row,
									6,
									design::row_highlight(ui, colors.hover, 1.0),
								);
							}
							let mut inner = ui.new_child(
								egui::UiBuilder::new()
									.max_rect(row.shrink2(egui::vec2(8.0, 4.0)))
									.layout(egui::Layout::left_to_right(egui::Align::Center)),
							);
							inner.spacing_mut().item_spacing.x = 12.0;
							inner.push_id(member.user.id.0, |ui| {
								let avatar = self.avatars.show(ui, &member.user, 32.0, state.demo);
								user_menu::show(
									&avatar,
									state,
									&member.user,
									&mut self.profile,
									&mut self.user_action,
								);
								if avatar.clicked() {
									self.profile = Some(member.user.clone());
								}
								if let Some(status) = status {
									design::presence_dot(
										ui,
										avatar.rect,
										profiles::presence_color(status),
										colors.sidebar,
									);
								}
								let text_color = if online {
									let role_color = guild.and_then(|guild| {
										state.member_roles(guild, member).1.map(|role| role.color)
									});
									let background = if response.hovered() || response.has_focus() {
										colors.hover
									} else {
										colors.sidebar
									};
									role_color.map_or(colors.text, |rgb| {
										design::role_name_color(rgb, background, colors.text)
									})
								} else {
									colors.muted
								};
								let mut show_name = |ui: &mut egui::Ui| {
									ui.allocate_ui_with_layout(
										egui::vec2(ui.available_width(), 18.0),
										egui::Layout::left_to_right(egui::Align::Center),
										|ui| {
											ui.spacing_mut().item_spacing.x = 5.0;
											let server_tag = member.user.primary_guild.as_deref();
											let trailing =
												profiles::server_tag_width(ui, server_tag)
													+ if server_tag.is_some() { 5.0 } else { 0.0 };
											account_badge::name(
												ui,
												&member.user,
												name,
												15.0,
												text_color,
												egui::Sense::hover(),
												trailing,
											);
											if let Some(tag) = server_tag {
												profiles::server_tag(
													ui,
													tag,
													&mut self.avatars,
													state.demo,
												);
											}
										},
									);
								};
								if let Some(subtitle) = subtitle {
									ui.vertical(|ui| {
										ui.spacing_mut().item_spacing.y = 1.0;
										show_name(ui);
										ui.add(
											egui::Label::new(
												RichText::new(subtitle)
													.size(12.0)
													.color(colors.muted),
											)
											.truncate()
											.selectable(false),
										);
									});
								} else {
									show_name(ui);
								}
							});
							user_menu::show(
								&response,
								state,
								&member.user,
								&mut self.profile,
								&mut self.user_action,
							);
							if response.clicked() {
								self.profile = Some(member.user.clone());
							}
						}
						None => {
							ui.allocate_exact_size(
								egui::vec2(ui.available_width(), 42.0),
								egui::Sense::hover(),
							);
						}
					}
				}
			},
		);
		ui.spacing_mut().item_spacing.y = row_spacing;
		if lazy && !visible.is_empty() {
			let first = visible.start;
			let mut last = visible.end.saturating_sub(1);
			if row_count < total
				&& output.state.offset.y > 0.0
				&& output.state.offset.y + output.inner_rect.height() >= output.content_size.y - 1.0
			{
				// Keep requesting the next chunk while at the bottom, without exposing
				// another page of empty rows before its first entry arrives.
				last = row_count;
				if state.member_slot(row_count).is_some()
					|| row_count
						.checked_sub(start)
						.is_some_and(|index| list.slots.get(index).is_some_and(Option::is_some))
				{
					self.member_extent = Some((member_key, (row_count + 100).min(total)));
					ui.ctx().request_repaint();
				}
			}
			if let Some(cmd) = state.focus_member_ranges(first, last) {
				commands.push(cmd);
			}
		}
	}
	/// Channel sidebar: context header, scrolling list and the account card.
	fn sidebar(
		&mut self,
		ui: &mut egui::Ui,
		state: &mut State,
		title: &str,
		commands: &mut Vec<Command>,
	) {
		let colors = design::palette(ui);
		egui::Panel::top("sidebar-header")
			.exact_size(48.0)
			.show_separator_line(false)
			.frame(egui::Frame::new().inner_margin(egui::Margin::symmetric(16, 0)))
			.show(ui, |ui| {
				let rect = ui.max_rect();
				ui.horizontal_centered(|ui| {
					if let Some(guild) = self.guild {
						self.server_menu.header(ui, state, guild, title);
						return;
					}
					ui.add(
						egui::Label::new(
							design::semibold(ui, title, 15.0).color(colors.text_strong),
						)
						.truncate()
						.selectable(false),
					);
				});
				ui.painter().hline(
					rect.x_range().expand(16.0),
					rect.bottom(),
					egui::Stroke::new(1.0, colors.border),
				);
			});
		egui::Frame::new()
			.inner_margin(egui::Margin {
				left: 8,
				right: 8,
				top: 8,
				bottom: 0,
			})
			.show(ui, |ui| {
				let ctx = ui.ctx().clone();
				if self.guild.is_none() {
					// Search and Friends share one row; Friends is the compact glyph beside it.
					ui.horizontal(|ui| {
						ui.spacing_mut().item_spacing.x = 6.0;
						let find = ui
							.add_enabled_ui(
								!self.ime_active
									&& !ctx.input(|input| {
										input
											.events
											.iter()
											.any(|event| matches!(event, egui::Event::Ime(_)))
									}),
								|ui| {
									ui.add_sized(
										[(ui.available_width() - 36.0).max(60.0), 30.0],
										egui::Button::new("Find conversation").truncate(),
									)
								},
							)
							.inner
							.on_hover_text("Search loaded conversations (Ctrl/Cmd+K)");
						find.widget_info(|| {
							egui::WidgetInfo::labeled(
								egui::Role::Button,
								ui.is_enabled(),
								"Find conversation, Ctrl or Command K",
							)
						});
						if find.clicked() {
							self.switcher.open(&ctx);
						}
						let friends = state.selected.is_none();
						let (rect, response) =
							ui.allocate_exact_size(egui::vec2(30.0, 30.0), egui::Sense::click());
						let hovered = response.hovered() || response.has_focus();
						if friends || hovered {
							ui.painter().rect_filled(
								rect,
								6,
								if friends {
									colors.selected
								} else {
									colors.hover
								},
							);
						}
						icons::paint(
							ui.painter(),
							icons::Icon::People,
							rect.shrink(7.0),
							if friends || hovered {
								colors.text_strong
							} else {
								colors.muted
							},
						);
						response.widget_info(|| {
							egui::WidgetInfo::selected(egui::Role::Button, true, friends, "Friends")
						});
						if response.on_hover_text("Friends").clicked() {
							state.open_home();
							self.guild = None;
							self.search.open = false;
						}
					});
					ui.add_space(8.0);
				}
				if let Some(guild) = self.guild
					&& state.can_open_member_settings(guild)
					&& state.server_members_shortcut(guild)
					&& ui
						.add_sized(
							[ui.available_width(), 32.0],
							egui::Button::new("Members").frame(false),
						)
						.clicked() && let Some(command) =
					self.preview_server_admin(state, guild, "members")
				{
					commands.push(command);
				}
				if !self.channel_preferences_status.is_empty() {
					ui.colored_label(design::palette(ui).warning, self.channel_preferences_status);
					if ui.button("Retry shortcuts").clicked() {
						if self.channel_preferences_loaded {
							self.channel_preferences_changed = true;
						} else {
							self.channel_preferences_reload = true;
						}
					}
				}
				let select = self.channel_list(ui, state);
				if let Some((channel, action)) = self.channel_move.take()
					&& let Some(command) = state.request_channel_action(channel, action)
				{
					commands.push(command);
				}
				if let Some(id) = select
					&& let Some(command) = state.select(id)
				{
					commands.push(command);
				}
				if let Some(parent) = self.archive_parent.take()
					&& let Some(command) =
						state.request_archives(parent, model::archives::Kind::Public, None)
				{
					self.search.open = false;
					self.archives.focus = true;
					commands.push(command);
				}
			});
	}
	/// Account card; while in a call it grows upward with the call header and quick actions.
	fn account_card(&mut self, ui: &mut egui::Ui, state: &mut State, commands: &mut Vec<Command>) {
		let colors = design::palette(ui);
		let mut anchor = None;
		let in_call = state.voice.active.is_some();
		egui::Frame::new()
			.fill(colors.raised)
			.corner_radius(8)
			.inner_margin(0)
			.show(ui, |ui| {
				ui.set_width(ui.available_width());
				ui.spacing_mut().item_spacing.y = 0.0;
				let divider = |ui: &mut egui::Ui| {
					let (line, _) = ui.allocate_exact_size(
						egui::vec2(ui.available_width(), 1.0),
						egui::Sense::hover(),
					);
					ui.painter().rect_filled(line, 0, colors.border);
				};
				if self.shows_update_banner() {
					self.update_banner(ui);
					divider(ui);
				}
				if in_call {
					self.voice_card_section(ui, state, commands);
					divider(ui);
				}
				egui::Frame::new()
					.inner_margin(egui::Margin::symmetric(8, 6))
					.show(ui, |ui| {
						ui.set_width(ui.available_width());
						ui.horizontal(|ui| {
							ui.spacing_mut().item_spacing.x = 8.0;
							if let Some(user) = &state.user {
								let avatar = self.avatars.show(ui, user, 32.0, state.demo);
								avatar.widget_info(|| {
									egui::WidgetInfo::labeled(
										egui::Role::Button,
										true,
										"Profile and status",
									)
								});
								if avatar.has_focus() {
									ui.painter().rect_stroke(
										avatar.rect.expand(2.0),
										4,
										egui::Stroke::new(1.0, colors.accent),
										egui::StrokeKind::Inside,
									);
								}
								user_menu::show(
									&avatar,
									state,
									user,
									&mut self.profile,
									&mut self.user_action,
								);
								design::presence_dot(
									ui,
									avatar.rect,
									profiles::presence_color(self.own_presence.status.wire()),
									colors.raised,
								);
								anchor = Some(avatar.on_hover_text("Profile and status"));
							}
							ui.with_layout(
								egui::Layout::right_to_left(egui::Align::Center),
								|ui| {
									ui.spacing_mut().item_spacing.x = 2.0;
									let settings =
										icons::button(ui, icons::Icon::Gear, 32.0, "User settings");
									if settings.clicked() {
										self.settings.open = true;
										egui::Popup::close_all(ui.ctx());
									}
									self.mute_toggle_with_settings(ui, state, commands, true);
									ui.add_space(4.0);
									self.mute_toggle_with_settings(ui, state, commands, false);
									ui.with_layout(
										egui::Layout::left_to_right(egui::Align::Center),
										|ui| {
											let identity = ui
												.vertical(|ui| {
													ui.spacing_mut().item_spacing.y = 0.0;
													ui.add(
														egui::Label::new(
															design::semibold(
																ui,
																state
																	.user
																	.as_ref()
																	.map_or("Your account", |u| {
																		u.name.as_str()
																	}),
																14.0,
															)
															.color(colors.text_strong),
														)
														.truncate()
														.selectable(false),
													);
													ui.add(
														egui::Label::new(
															RichText::new(
																if !self
																	.own_presence
																	.custom_status
																	.is_empty()
																{
																	self.own_presence
																		.custom_status
																		.clone()
																} else if let Some(game) = self
																	.own_game
																	.as_deref()
																	.filter(|_| {
																		self.share_game_activity
																	}) {
																	game.to_owned()
																} else if state.demo {
																	"Offline preview".to_owned()
																} else if state.gateway_connected {
																	self.own_presence
																		.status
																		.label()
																		.to_owned()
																} else {
																	"Reconnecting…".to_owned()
																},
															)
															.size(12.0)
															.color(colors.muted),
														)
														.truncate()
														.selectable(false),
													);
												})
												.response;
											let identity = ui
												.interact(
													identity.rect,
													ui.scope_id().with("account-identity"),
													egui::Sense::click(),
												)
												.on_hover_text("Profile and status");
											identity.widget_info(|| {
												egui::WidgetInfo::labeled(
													egui::Role::Button,
													true,
													"Profile and status",
												)
											});
											if let Some(avatar) = anchor.take() {
												if identity.has_focus() {
													ui.painter().rect_stroke(
														identity.rect.expand(2.0),
														4,
														egui::Stroke::new(1.0, colors.accent),
														egui::StrokeKind::Inside,
													);
												}
												anchor = Some(avatar.union(identity));
											}
										},
									);
								},
							);
						});
					});
			});
		if let Some(anchor) = anchor {
			self.account_menu(&anchor, state, commands);
		}
	}
	/// Conversation header: channel identity on the left, tools and search on the right.
	fn channel_header(
		&mut self,
		ui: &mut egui::Ui,
		state: &mut State,
		selected_voice: bool,
		show_members: bool,
		wide_members: bool,
		commands: &mut Vec<Command>,
	) {
		let colors = design::palette(ui);
		egui::Panel::top("channel-header")
			.exact_size(48.0)
			.show_separator_line(false)
			.frame(
				egui::Frame::new()
					.fill(design::section_surface(
						ui,
						design::window_palette(ui).chat,
						design::ImageSection::TopBar,
					))
					.inner_margin(egui::Margin::symmetric(16, 0)),
			)
			.show(ui, |ui| {
				let rect = ui.max_rect();
				ui.painter().hline(
					rect.x_range().expand(16.0),
					rect.bottom(),
					egui::Stroke::new(1.0, colors.border),
				);
				let channel = state.selected.and_then(|id| state.channel(id)).cloned();
				let dm = channel
					.as_ref()
					.is_some_and(|c| c.kind == 1 && c.guild.is_none());
				let shortcuts_available = self.shortcuts_available(state);
				ui.horizontal_centered(|ui| {
					ui.spacing_mut().item_spacing.x = 8.0;
					match channel.as_ref() {
						Some(c) if c.guild.is_none() && c.kind == 3 => {
							let avatar = self.avatars.show_group(ui, c, 24.0, state.demo);
							self.group_menu.context(
								&avatar,
								state,
								c,
								shortcuts::ShortcutView::new(
									&self.channel_preferences,
									shortcuts_available,
								),
							);
						}
						Some(c) if c.guild.is_none() => {
							if let Some(user) = c.recipients.first() {
								// Header avatar identifies the conversation; the profile is a
								// context-menu action, not a click target.
								let avatar = self.avatars.show_plain(ui, user, 24.0, state.demo);
								if dm {
									user_menu::show_with_pin(
										&avatar,
										state,
										user,
										&mut self.profile,
										&mut self.user_action,
										Some(shortcuts::ShortcutView::new(
											&self.channel_preferences,
											shortcuts_available,
										)),
									);
								}
								if dm
									&& let Some(status) = profiles::presence(state, user.id, None).0
								{
									design::presence_dot(
										ui,
										avatar.rect,
										profiles::presence_color(status),
										colors.sidebar,
									);
								}
							} else {
								icons::inline(ui, icons::Icon::People, 22.0, colors.muted);
							}
						}
						Some(c) => {
							let icon = match c.kind {
								2 | 13 => icons::Icon::Speaker,
								5 => icons::Icon::Megaphone,
								15 | 16 => icons::Icon::Forum,
								10..=12 => icons::Icon::Thread,
								_ => icons::Icon::Hash,
							};
							icons::inline(ui, icon, 22.0, colors.muted);
						}
						None => {}
					}
					ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
						ui.spacing_mut().item_spacing.x = 4.0;
						if selected_voice
							&& icons::toggle(
								ui,
								icons::Icon::Forum,
								32.0,
								self.voice_chat_open,
								if self.voice_chat_open {
									"Hide chat"
								} else {
									"Show chat"
								},
							)
							.clicked()
						{
							self.voice_chat_open = !self.voice_chat_open;
							self.focus_switched_composer = self.voice_chat_open;
						}
						if let Some(c) = channel
							.as_ref()
							.filter(|c| c.guild.is_none() && c.kind == 3)
						{
							self.group_menu.dropdown(
								ui,
								state,
								c,
								shortcuts::ShortcutView::new(
									&self.channel_preferences,
									shortcuts_available,
								),
							);
						}
						if state.selected.is_some() && !selected_voice {
							if self.search.open && !self.search.pins() {
								ui.allocate_ui_with_layout(
									egui::vec2(
										240.0_f32.min(ui.available_width() * 0.5).max(120.0),
										28.0,
									),
									egui::Layout::left_to_right(egui::Align::Center),
									|ui| self.search.header_input(ui, state, commands),
								);
							} else {
								// Search pill.
								let (pill, response) = ui.allocate_exact_size(
									egui::vec2(144.0, 28.0),
									egui::Sense::click(),
								);
								let enabled = state.can_search();
								response.widget_info(|| {
									egui::WidgetInfo::labeled(egui::Role::Button, enabled, "Search")
								});
								ui.painter().rect_filled(pill, 6, colors.raised);
								let pill_text = if enabled {
									colors.muted
								} else {
									colors.muted.gamma_multiply(0.5)
								};
								ui.painter().text(
									pill.left_center() + egui::vec2(10.0, 0.0),
									egui::Align2::LEFT_CENTER,
									"Search",
									egui::FontId::proportional(13.0),
									pill_text,
								);
								icons::paint(
									ui.painter(),
									icons::Icon::Search,
									egui::Rect::from_center_size(
										pill.right_center() - egui::vec2(14.0, 0.0),
										egui::Vec2::splat(16.0),
									),
									pill_text,
								);
								if enabled
									&& response.on_hover_text("Search this conversation").clicked()
								{
									if state.archives.is_some() {
										commands.push(state.clear_archives());
									}
									self.search.toggle(false);
								}
							}
							ui.add_space(4.0);
							if icons::toggle(
								ui,
								icons::Icon::People,
								32.0,
								show_members,
								"Show member list",
							)
							.clicked()
							{
								if wide_members {
									self.reading_preferences.show_members =
										!self.reading_preferences.show_members;
								} else {
									self.members_narrow_open = !self.members_narrow_open;
								}
							}
							let pins_open = self.search.open && self.search.pins();
							let pins = ui
								.add_enabled_ui(state.can_search() || pins_open, |ui| {
									icons::toggle(
										ui,
										icons::Icon::Pin,
										32.0,
										pins_open,
										"Pinned messages",
									)
								})
								.inner;
							if pins.clicked()
								&& self.search.toggle(true)
								&& let Some(command) = state.request_pins()
							{
								commands.push(command);
							}
							self.pins_anchor =
								(self.search.open && self.search.pins()).then_some(pins.rect);
							if let Some(c) = channel
								.as_ref()
								.filter(|c| c.guild.is_some() && matches!(c.kind, 0 | 5 | 15 | 16))
							{
								let allowed =
									state.can_archive(c.id, model::archives::Kind::Public);
								let archive = ui
									.add_enabled_ui(allowed, |ui| {
										icons::button(ui, icons::Icon::Thread, 32.0, "Threads")
									})
									.inner;
								if archive.clicked() {
									self.archive_parent = Some(c.id);
								}
							}
							let reload = ui
								.add_enabled_ui(
									state.freshness != Freshness::Loading
										&& state
											.selected
											.is_some_and(|id| state.can_read_history(id)),
									|ui| {
										icons::button(
											ui,
											icons::Icon::Reload,
											32.0,
											"Reload history",
										)
									},
								)
								.inner;
							if reload.clicked() {
								self.timeline.follow_latest();
								commands.push(state.history(None));
							}
						}
						if let Some(channel) = state.selected.filter(|_| {
							channel
								.as_ref()
								.is_some_and(|c| c.guild.is_none() && matches!(c.kind, 1 | 3))
						}) {
							self.voice_settings(ui, state.demo, state.voice.active.is_some());
							self.call_button(ui, state, channel, commands);
						}
						ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
							// Centre the name block in the fixed-height header even without a subtitle.
							let name = channel
								.as_ref()
								.map_or("Direct Messages", |c| state.conversation_name(c));
							let subtitle = channel
								.as_ref()
								.filter(|_| dm)
								.and_then(|c| c.recipients.first())
								.and_then(|user| {
									let (_, custom, activities) =
										profiles::presence(state, user.id, None);
									profiles::subtitle(custom, activities)
								});
							let name_height = ui
								.painter()
								.layout_no_wrap(
									name.to_owned(),
									egui::FontId::new(16.0, design::semibold_family(ui.ctx())),
									colors.text_strong,
								)
								.size()
								.y;
							let subtitle_height = subtitle.as_ref().map_or(0.0, |text| {
								1.0 + ui
									.painter()
									.layout_no_wrap(
										text.clone(),
										egui::FontId::proportional(12.0),
										colors.muted,
									)
									.size()
									.y
							});
							ui.vertical(|ui| {
								ui.add_space(
									((ui.available_height() - name_height - subtitle_height) / 2.0)
										.max(0.0),
								);
								ui.spacing_mut().item_spacing.y = 1.0;
								ui.add(
									egui::Label::new(
										design::semibold(ui, name, 16.0).color(colors.text_strong),
									)
									.truncate(),
								);
								{
									if let Some(text) = subtitle {
										ui.add(
											egui::Label::new(
												RichText::new(&text).size(12.0).color(colors.muted),
											)
											.truncate(),
										)
										.on_hover_text(text);
									}
								}
							});
							let in_call = state.voice.active.as_ref().is_some_and(|call| {
								Some(call.channel) == state.selected
									&& matches!(
										call.phase,
										client_core::voice::Phase::Connected
											| client_core::voice::Phase::Waiting
									)
							});
							if in_call {
								ui.add_space(4.0);
								icons::inline(ui, icons::Icon::InCall, 16.0, colors.positive);
								ui.add(
									egui::Label::new(
										design::medium(ui, "In a call", 14.0)
											.color(colors.positive),
									)
									.selectable(false),
								);
							}
						});
					});
				});
			});
	}
	fn clear_draft(&mut self, state: &mut State, channel: Id) {
		if self.draft_restore_pending {
			// Preserve an explicit clear until the outstanding disk snapshot is merged.
			state.drafts.insert(channel, String::new());
		} else {
			state.drafts.remove(&channel);
		}
		self.draft_changes.push(channel);
	}
	fn restore_pending(&mut self, state: &mut State, channel: Id, nonce: &str) {
		if let Some(index) = state.pending.iter().position(|p| {
			p.nonce == nonce && p.channel == channel && p.delivery != model::Delivery::Sending
		}) {
			if !state.drafts.contains_key(&channel) && state.drafts.len() >= 64 {
				state.status =
					"Draft budget full. Clear an existing draft before restoring pending text";
			} else if state.drafts.get(&channel).is_none_or(String::is_empty) {
				let pending = state.pending.remove(index);
				if !pending.attachments.is_empty() {
					state.status = "Text restored; reselect the attachment before sending again";
				}
				state.drafts.insert(channel, pending.content);
				self.draft_changes.push(channel);
			} else {
				state.status = "Keep or clear the existing draft before restoring pending text";
			}
		}
	}
	fn composer(
		&mut self,
		ui: &mut egui::Ui,
		state: &mut State,
		channel: Id,
		ctx: &egui::Context,
		commands: &mut Vec<Command>,
	) {
		let had_edit = self.editing.is_some();
		self.reconcile_edit(state);
		let closed_here = self.edit_closed_channel.take() == Some(channel);
		if closed_here || (had_edit && self.editing.is_none()) {
			egui::text_edit::TextEditState::default()
				.store(ctx, ui.make_persistent_id("message-edit"));
			ui.weak("Message deleted. The unchanged edit was closed.");
			ctx.request_repaint();
			return;
		}
		let colors = crate::design::palette(ui);
		let keyboard_enabled = !self.switcher_frame
			&& !self.switcher.is_open()
			&& ctx.memory(|memory| memory.top_modal_layer().is_none());
		fn ime_updates_text(event: &egui::Event) -> bool {
			match event {
				egui::Event::Ime(
					egui::ImeEvent::Preedit { text, .. } | egui::ImeEvent::Commit(text),
				) => !text.is_empty(),
				egui::Event::Ime(egui::ImeEvent::DeleteSurrounding { .. }) => true,
				_ => false,
			}
		}
		if keyboard_enabled
			&& self.editing.is_none()
			&& !self.ime_active
			&& !egui::Popup::is_any_open(ctx)
			&& ctx.memory(|memory| memory.has_focus(ui.make_persistent_id("message-input")))
			&& state.drafts.get(&channel).is_none_or(String::is_empty)
			&& ctx.input_mut(|input| {
				!input.events.iter().any(ime_updates_text)
					&& crate::keybinds::pressed_exact(
						input,
						self.keybinds.chord(model::KeybindAction::EditLastMessage),
					)
			}) && let Some(message) = state
			.timeline
			.iter()
			.rev()
			.find(|message| !message.unsupported && state.can_edit(channel, message.id))
		{
			self.editing = Some((channel, message.id, message.content.clone()));
			self.edit_modified = None;
			self.composer_edit = None;
		}
		let editing_key = self
			.editing
			.as_ref()
			.filter(|(c, _, _)| *c == channel)
			.map(|(c, id, _)| (*c, *id));
		let editing_here = editing_key.is_some();
		if let Some((_, id)) = editing_key {
			if state.timeline.get(id).is_some() {
				self.edit_undo_cleared = false;
			} else if !self.edit_undo_cleared {
				let editor_id = ui.make_persistent_id("message-edit");
				if let Some(mut editor) = egui::text_edit::TextEditState::load(ctx, editor_id) {
					editor.clear_undoer();
					editor.store(ctx, editor_id);
				}
				self.edit_undo_cleared = true;
			}
		}
		if !editing_here
			&& !state.can_compose(channel)
			&& !state.can_request_application_commands(channel)
		{
			if self.pending_mention.take().is_some() {
				state.status = "You don't have permission to mention anyone in this channel.";
			}
			// Returning to an edit must restore its focus after visiting a read-only channel.
			self.composer_edit = None;
			self.mention_menu = mentions::Menu::default();
			self.slash_commands.suspend(state, channel);
			self.emoji_picker = emoji_picker::Picker::default();
			self.ime_active = false;
			self.focus_switched_composer = false;
			egui::Frame::new()
				.fill(colors.raised)
				.corner_radius(8)
				.inner_margin(12)
				.show(ui, |ui| {
					let mut hint =
						"You don't have permission to send messages in this channel.".to_owned();
					ui.add_enabled(
						false,
						TextEdit::singleline(&mut hint)
							.desired_width(f32::INFINITY)
							.frame(egui::Frame::NONE),
					);
				});
			return;
		}
		let focus_edit =
			keyboard_enabled && editing_key.is_some() && self.composer_edit != editing_key;
		let focus_composer = keyboard_enabled
			&& (std::mem::take(&mut self.focus_switched_composer)
				|| (self.composer_edit != editing_key
					&& (editing_here || self.composer_edit.is_some_and(|(c, _)| c == channel))));
		if self.composer_edit != editing_key {
			self.composer_edit = editing_key;
			self.edit_sent = false;
			self.ime_active = false;
			self.mention_menu = mentions::Menu::default();
			self.emoji_picker = emoji_picker::Picker::default();
		}
		let mut cancel_edit = false;
		if ctx.input(|input| !input.raw.hovered_files.is_empty()) {
			let available = state.can_attach(channel) && !self.upload_busy && !editing_here;
			egui::Frame::new()
				.fill(colors.accent.gamma_multiply(0.12))
				.stroke(egui::Stroke::new(1.5, colors.accent))
				.corner_radius(12)
				.inner_margin(20)
				.show(ui, |ui| {
					ui.set_min_width((ui.available_width() - 2.0).max(0.0));
					ui.label(design::semibold(
						ui,
						if available {
							"Drop files to attach"
						} else {
							"Attachments unavailable right now"
						},
						18.0,
					));
					ui.add_space(6.0);
					ui.label(
						egui::RichText::new(if available {
							"Up to 10 files · 500 MB max · Account limit applies"
						} else {
							"Return to an available conversation after the current operation finishes"
						})
						.color(colors.muted),
					);
				});
			ui.add_space(8.0);
		}
		// Reply/edit context sits flush on top of the input as one rounded block, like Discord.
		let mut cap_top: Option<f32> = None;
		if editing_here {
			let unavailable = editing_key.is_some_and(|(_, id)| state.timeline.get(id).is_none());
			let cap = composer_cap(ui, &colors, |ui| {
				ui.label(
					RichText::new(if unavailable {
						"Message unavailable · unsent edit"
					} else {
						"Editing message"
					})
					.size(13.0)
					.color(colors.muted),
				);
				if self.edit_sent {
					ui.label(
						RichText::new("· Save requested, check the connection before retrying")
							.size(12.0)
							.color(colors.muted),
					);
				}
				ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
					cancel_edit =
						icons::button(ui, icons::Icon::Close, 22.0, "Cancel edit").clicked();
					if unavailable
						&& ui
							.add(
								egui::Button::new(
									RichText::new("Copy edit text")
										.size(12.0)
										.color(colors.muted),
								)
								.frame(false),
							)
							.clicked() && let Some((_, _, text)) = &self.editing
					{
						ui.ctx().copy_text(text.clone());
					}
				});
			});
			cap_top = Some(cap.top());
		} else if let Some(reply) = state.reply {
			let author = state
				.timeline
				.get(reply.target())
				.map_or("an earlier message", |message| message.author.name.as_str())
				.to_owned();
			let cap = composer_cap(ui, &colors, |ui| {
				ui.spacing_mut().item_spacing.x = 0.0;
				ui.label(RichText::new("Replying to ").size(13.0).color(colors.muted));
				ui.label(design::semibold(ui, author.as_str(), 13.0).color(colors.text_strong));
				ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
					ui.spacing_mut().item_spacing.x = 6.0;
					if icons::button(ui, icons::Icon::Close, 22.0, "Cancel reply").clicked() {
						state.reply = None;
					}
					if let Some(reply) = state.reply.as_mut() {
						mention_switch(ui, &colors, &mut reply.mention);
					}
					if ui
						.add_enabled(
							state.can_open_reply_target(reply.target()),
							egui::Button::new(
								RichText::new("View original")
									.size(12.0)
									.color(colors.muted),
							)
							.frame(false),
						)
						.on_disabled_hover_text(if state.timeline.is_deleted(reply.target()) {
							"The original message was deleted"
						} else {
							"Wait for readable, current message history"
						})
						.clicked()
					{
						self.timeline.request_reply_target(reply.target());
					}
				});
			});
			cap_top = Some(cap.top());
		}
		let full = state.draft_bytes() >= MAX_DRAFT_BYTES
			|| (!state.drafts.contains_key(&channel) && state.drafts.len() >= 64);
		if full && !editing_here {
			ui.label("Draft budget full. Clear an existing draft to continue.");
			if state.drafts.contains_key(&channel) && ui.button("Clear this draft").clicked() {
				self.clear_draft(state, channel);
			}
			return;
		}
		// Wayland/Fcitx can repeatedly emit empty preedits while idle. Only real
		// composition blocks shortcuts; dismissing an active composition still does.
		let ime_this_frame = keyboard_enabled
			&& (self.ime_active || ctx.input(|i| i.events.iter().any(ime_updates_text)));
		if keyboard_enabled {
			ctx.input(|i| {
				for event in &i.events {
					if let egui::Event::Ime(event) = event {
						match event {
							egui::ImeEvent::Preedit { text, .. } => {
								self.ime_active = !text.is_empty()
							}
							egui::ImeEvent::Commit(_) => self.ime_active = false,
							_ => {}
						}
					}
				}
			});
		}
		let composer_id = ui.make_persistent_id(if editing_here {
			"message-edit"
		} else {
			"message-input"
		});
		if editing_here {
			self.edit_widget_id = Some(composer_id);
		}
		if focus_composer {
			ctx.memory_mut(|m| m.request_focus(composer_id));
		}
		if focus_edit {
			let mut edit_state = egui::text_edit::TextEditState::default();
			let count = self
				.editing
				.as_ref()
				.map_or(0, |(_, _, content)| content.chars().count());
			edit_state
				.cursor
				.set_char_range(Some(egui::text::CCursorRange::one(
					egui::text::CCursor::new(count),
				)));
			edit_state.store(ctx, composer_id);
		}
		let pasted_text = self.pasted_text.take();
		let paste_key_released = ctx.input(|input| {
			input.events.iter().any(|event| {
				matches!(
					event,
					egui::Event::Key {
						key: egui::Key::V,
						pressed: false,
						..
					}
				)
			})
		});
		let paste_enabled = keyboard_enabled
			&& !editing_here
			&& self.slash_commands.active.is_none()
			&& !self.ime_active
			&& !ime_this_frame
			&& ctx.memory(|m| m.has_focus(composer_id));
		if let Some((paste_channel, target, text)) = pasted_text {
			if paste_enabled && paste_channel == channel && target == composer_id {
				ctx.input_mut(|i| i.events.push(egui::Event::Paste(text)));
				self.pasted_text_frame = Some(ctx.cumulative_frame_nr());
			} else {
				state.status = "Paste cancelled; focus the message and paste again";
			}
		} else if paste_enabled && self.pasted_text_frame != Some(ctx.cumulative_frame_nr()) {
			let mut request = AttachmentPaste {
				target: composer_id,
				text: None,
				image: None,
			};
			let mut requested = false;
			ctx.input_mut(|input| {
				// eframe consumes native paste key-down. File-only clipboards can emit
				// no Paste event, so the matching key-up is also a paste trigger.
				let shortcut = input.events.iter().any(|event| {
					matches!(event,
					egui::Event::Key { key: egui::Key::V, pressed, repeat: false, modifiers, .. }
					if !modifiers.shift && (modifiers.ctrl || modifiers.command || modifiers.alt)
						&& (*pressed || !self.paste_key_handled))
				});
				let has_paste = input.events.iter().any(|event| {
					matches!(event, egui::Event::Paste(_) | egui::Event::PasteImage(_))
				});
				if has_paste || shortcut {
					requested = true;
					self.paste_key_handled = true;
					input.events.retain_mut(|event| match event {
						egui::Event::Paste(text) => {
							request.text = Some(std::mem::take(text));
							false
						}
						egui::Event::PasteImage(image) => {
							request.image = Some(image.clone());
							false
						}
						egui::Event::Key {
							key: egui::Key::V, ..
						} => false,
						// Option+V can also produce a platform text character.
						egui::Event::Text(_) if shortcut => false,
						_ => true,
					});
				}
			});
			if requested {
				if request
					.text
					.as_ref()
					.is_some_and(|text| text.len() > MAX_DRAFT_BYTES)
				{
					state.status = "Pasted text exceeds the draft limit";
				} else if request
					.image
					.as_ref()
					.is_some_and(|image| image.pixels.len() > 4 * 1024 * 1024)
				{
					state.status = "Paste an image with at most 4 million pixels";
				} else {
					self.attachment_paste_requested = Some(request);
					self.upload_busy = true;
				}
			}
		}
		if paste_key_released {
			self.paste_key_handled = false;
		}
		if !editing_here
			&& state
				.drafts
				.get(&channel)
				.is_some_and(|draft| draft.starts_with('/'))
			&& let Some(command) = state.request_application_commands(channel, false)
		{
			commands.push(command);
		}
		let composer_content = if editing_here {
			self.editing
				.as_ref()
				.map_or("", |(_, _, text)| text.as_str())
		} else {
			state.drafts.get(&channel).map_or("", String::as_str)
		};
		let count_before = composer_content.chars().count();
		// Suggestion rows can take focus on press; keep the editor alive until release
		// so the shared member/channel/emoji popup can finish the click.
		let suggestion_pointer = self.mention_menu.pointer_interacting(ctx, channel)
			|| (self.slash_commands.active.is_none()
				&& self.slash_commands.pointer_interacting(ctx, channel));
		if keyboard_enabled && !self.ime_active && !ime_this_frame && suggestion_pointer {
			ctx.memory_mut(|memory| memory.request_focus(composer_id));
		}
		let mention_enabled = keyboard_enabled
			&& !self.ime_active
			&& !ime_this_frame
			&& ctx.memory(|m| m.has_focus(composer_id));
		self.slash_commands.refresh(
			state,
			channel,
			composer_content,
			!editing_here
				&& keyboard_enabled
				&& !self.ime_active
				&& !ime_this_frame
				&& (mention_enabled || self.slash_commands.active.is_some()),
		);
		let slash_pick = if !editing_here
			&& keyboard_enabled
			&& !self.ime_active
			&& !ime_this_frame
			&& (mention_enabled || self.slash_commands.form_has_focus(ctx))
		{
			self.slash_commands.keys(ctx)
		} else {
			None
		};
		let mention_users = if mention_enabled || composer_content.contains("<@") {
			mentions::known_users(state, channel)
		} else {
			Vec::new()
		};
		let mass_mentions =
			state.permission(channel, model::permissions::MENTION_EVERYONE) == Some(true);
		let cursor = egui::text_edit::TextEditState::load(ctx, composer_id)
			.and_then(|s| s.cursor.char_range())
			.filter(|r| r.is_empty())
			.map(|r| r.primary.index.0);
		self.mention_menu.refresh(
			state,
			channel,
			composer_content,
			cursor.filter(|_| mention_enabled),
			&mention_users,
		);
		let mention_pick = if mention_enabled {
			self.mention_menu.keys(ctx)
		} else {
			None
		};
		let mut editing = if editing_here {
			self.editing.take()
		} else {
			None
		};
		let demo = state.demo;
		let enter = keyboard_enabled
			&& !self.ime_active
			&& !ime_this_frame
			&& ctx.memory(|m| m.has_focus(composer_id))
			&& ctx.input_mut(|i| {
				crate::keybinds::pressed_exact(
					i,
					self.keybinds.chord(model::KeybindAction::SendMessage),
				)
			});
		let placeholder = state.channel(channel).map_or_else(
			|| "Message".to_owned(),
			|c| {
				if c.guild.is_some() {
					format!("Message #{}", c.name)
				} else {
					format!("Message @{}", state.conversation_name(c))
				}
			},
		);
		let can_attach = !editing_here
			&& self.slash_commands.active.is_none()
			&& state.can_attach(channel)
			&& !self.upload_busy
			&& self.attachment_files.len() < 10;
		let application_command = !editing_here && self.slash_commands.active.is_some();
		let can_send = if let Some((edit_channel, message)) = editing_key {
			state.freshness == Freshness::Fresh
				&& state.can_edit(edit_channel, message)
				&& count_before > 0
		} else if application_command {
			self.slash_commands.can_submit(state, channel)
		} else {
			state.can_send(channel)
				&& (self.attachment.is_none() || state.can_attach(channel))
				&& !self.upload_busy
				&& !(state.demo && self.attachment.is_some())
				&& (count_before > 0 || self.attachment.is_some())
		};
		if cap_top.is_some() {
			// The cap and the input form one block: undo the automatic vertical item gap.
			ui.add_space(-ui.spacing().item_spacing.y);
		}
		egui::Frame::new()
            .fill(colors.raised)
            .corner_radius(if cap_top.is_some() {
                egui::CornerRadius { nw: 0, ne: 0, sw: 8, se: 8 }
            } else {
                egui::CornerRadius::same(8)
            })
            .inner_margin(egui::Margin::symmetric(10, 6))
            .show(ui, |ui| {
                // Outer frame bounds for the autocomplete popout: undo the inner margin and
                // include the reply/edit cap so the popout never covers it.
                let composer_anchor = egui::Rect::from_min_max(
                    egui::pos2(ui.max_rect().left() - 10.0, cap_top.unwrap_or(ui.max_rect().top() - 6.0)),
                    egui::pos2(ui.max_rect().right() + 10.0, ui.max_rect().top()),
                );
                if !editing_here && self.attachment.is_some() {
                    self.attachment_tray(ui);
                }
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    let attach = ui
                        .add_enabled_ui(can_attach, |ui| {
                            icons::button(ui, icons::Icon::Attach, 28.0, "Attach files")
                        })
                        .inner
						.on_hover_text("Choose, drop, or paste files (Ctrl/Cmd/Option+V). Up to 10 files and 500 MB total; account limits may be lower. Send starts the upload.");
                    if !editing_here { self.extensions.composer_menu(ui, state); }
                    if attach.clicked() {
                        self.attach_requested = true;
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.spacing_mut().item_spacing.x = 4.0;
                        let send_button = ui
                            .add_enabled_ui(can_send, |ui| {
                                icons::button(
                                    ui,
                                    icons::Icon::Send,
                                    28.0,
                                    if editing_here { "Save edit" } else if application_command { "Send command" } else { "Send message" },
                                )
                            })
                            .inner;
                        let send = enter || send_button.clicked();
                        if count_before + 200 >= MAX_CONTENT {
                            ui.label(
                                RichText::new(format!("{}", MAX_CONTENT.saturating_sub(count_before)))
                                    .size(11.0)
                                    .color(if count_before >= MAX_CONTENT { colors.danger } else { colors.muted }),
                            );
                        }
                        let pick = ui
                            .add_enabled_ui(!application_command && !self.ime_active && !ime_this_frame, |ui| {
                                self.emoji_picker.image_sharing_enabled = self.image_sharing_enabled;
                                self.emoji_picker
                                    .show(ui, state, channel, &mut self.avatars, commands)
                            })
                            .inner;
                        // A chosen GIF is its own message; the typed draft stays untouched.
                        if self.emoji_picker.is_open() {
                            self.reaction_picker.dismiss(state, commands);
                        }
                        let pick = match pick {
                            Some(emoji_picker::Pick::Insert(text)) => {
                                self.reaction_picker.record(&text);
                                Some(text)
                            },
                            Some(emoji_picker::Pick::React(_, _)) => None,
                            Some(emoji_picker::Pick::Image(asset)) => {
                                if editing_here { state.status = "Finish or cancel the edit before attaching an image."; }
                                else if self.upload_busy { state.status = "Wait for the upload before attaching an image."; }
                                else if self.image_sharing_enabled && state.can_send(channel) && state.can_attach(channel) { self.image_share_requested = Some(asset); }
                                None
                            },
                            Some(emoji_picker::Pick::Sticker(sticker)) => {
                                if editing_here { state.status = "Finish or cancel the edit before sending a sticker."; }
                                else if self.upload_busy { state.status = "Wait for the upload before sending a sticker."; }
                                else if let Some(command) = state.prepare_sticker_send(&sticker) { self.timeline.follow_latest(); commands.push(command); }
                                None
                            },
                            Some(emoji_picker::Pick::Send(url)) => {
                                if editing_here {
                                    state.status = "Finish or cancel the edit before sending a GIF.";
                                } else if self.upload_busy || !state.can_send(channel) {
                                    state.status = "Sending is unavailable with the current connection or permissions";
                                } else {
                                    let saved = state.drafts.insert(channel, url);
                                    let command = state.prepare_send();
                                    match saved {
                                        Some(saved) => {
                                            state.drafts.insert(channel, saved);
                                        }
                                        None => {
                                            state.drafts.remove(&channel);
                                        }
                                    }
                                    if let Some(command) = command {
                                        self.timeline.follow_latest();
                                        commands.push(command);
                                    }
                                }
                                None
                            }
                            None => None,
                        };
                        let edit = ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                            ui.vertical(|ui| {
                                ui.set_width(ui.available_width());
                        if application_command {
                            return ui.scope(|ui| {
                                ui.add_enabled_ui(keyboard_enabled, |ui| {
                                    self.slash_commands.composer(ui, state, channel, self.keybinds.chord(model::KeybindAction::SendMessage));
                                });
                                if self.slash_commands.active.is_none() {
                                    ctx.memory_mut(|memory| memory.request_focus(composer_id));
                                }
                                self.slash_commands.show(ui, composer_anchor, state, channel, &mut self.avatars);
                            }).response;
                        }
                        let composer_escape = keyboard_enabled
                            && !self.ime_active
                            && !ime_this_frame
                            && ctx.memory(|m| {
                                m.has_focus(composer_id) || m.had_focus_last_frame(composer_id)
                            });
                        cancel_edit |= editing_here
                            && composer_escape
                            && ctx.input_mut(|i| {
                                i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)
                            });
                        if !editing_here
                            && state.reply.is_some()
                            && composer_escape
                            && ctx.input_mut(|i| {
                                i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)
                            })
                        {
                            state.reply = None;
                        }
                        let remaining = if editing_here { MAX_CONTENT * 4 } else { MAX_DRAFT_BYTES.saturating_sub(state.draft_bytes()) };
                        // Temporarily own the buffer so suggestions can borrow the current
                        // permission state without cloning the draft or server catalogs.
                        let restore_empty_draft = self.draft_restore_pending && state.drafts.contains_key(&channel);
                        let mut new_draft = if editing_here { String::new() } else {
                            state.drafts.remove(&channel).unwrap_or_default()
                        };
                        let draft = if let Some((_, _, content)) = &mut editing {
                            content
                        } else {
                            &mut new_draft
                        };
						let mut mention_changed = false;
						match self.apply_pending_mention(ctx, composer_id, draft, remaining) {
							None => {}
							Some(MentionWrite::Inserted) => mention_changed = true,
							Some(MentionWrite::DidNotFit) => {
								state.status = "Mention will not fit. Shorten this message or free draft space.";
							}
						}
						if let Some(pick) = pick {
                            let mut edit_state =
                                egui::text_edit::TextEditState::load(ctx, composer_id).unwrap_or_default();
                            let range = edit_state.cursor.char_range();
                            if let Some(cursor) = emoji_picker::insert(draft, &pick, range, remaining) {
                                edit_state
                                    .cursor
                                    .set_char_range(Some(egui::text::CCursorRange::one(
                                        egui::text::CCursor::new(cursor),
                                    )));
                                edit_state.store(ctx, composer_id);
                                mention_changed = true;
                            } else {
                                state.status =
                                    "Emoji will not fit. Shorten this message or free draft space.";
                            }
                        }
                        if let Some(pick) = slash_pick
                            && let Some(cursor) = self.slash_commands.accept(pick, draft, remaining) {
                            let mut edit_state = egui::text_edit::TextEditState::load(ctx, composer_id).unwrap_or_default();
                            edit_state.cursor.set_char_range(Some(egui::text::CCursorRange::one(egui::text::CCursor::new(cursor))));
                            edit_state.store(ctx, composer_id);
                            mention_changed = true;
                        }
                        if let Some(pick) = mention_pick
                            && let Some(cursor) = mentions::insert(draft, pick)
                        {
                            let mut edit_state =
                                egui::text_edit::TextEditState::load(ctx, composer_id).unwrap_or_default();
                            edit_state
                                .cursor
                                .set_char_range(Some(egui::text::CCursorRange::one(
                                    egui::text::CCursor::new(cursor),
                                )));
                            edit_state.store(ctx, composer_id);
                            mention_changed = true;
                        }
						if mention_enabled
							&& let Some(cursor) = cursor
							&& let Some(cursor) =
								emoji_picker::complete_shortcode(draft, cursor, remaining)
						{
							let mut edit_state = egui::text_edit::TextEditState::load(ctx, composer_id)
								.unwrap_or_default();
							edit_state.cursor.set_char_range(Some(egui::text::CCursorRange::one(
								egui::text::CCursor::new(cursor),
							)));
							edit_state.store(ctx, composer_id);
							mention_changed = true;
						}
                        if keyboard_enabled
                            && !self.ime_active
                            && !ime_this_frame
                            && ctx.memory(|m| m.has_focus(composer_id))
							&& let Some(style) = formatting::Style::consume(ctx, &self.keybinds)
                        {
                            let mut edit_state =
                                egui::text_edit::TextEditState::load(ctx, composer_id).unwrap_or_default();
                            let range = edit_state.cursor.char_range();
                            if let Some(range) = formatting::apply(draft, style, range, remaining) {
                                edit_state.cursor.set_char_range(Some(range));
                                edit_state.store(ctx, composer_id);
                                mention_changed = true;
                            } else {
                                state.status = "Formatting will not fit. Shorten this message or free draft space.";
                            }
                        }
                        let rich_layout = &mut self.composer_layout;
                        if !self.ime_active
                            && !ime_this_frame
                            && ctx.input(|i| {
                                i.key_pressed(egui::Key::Backspace) || i.key_pressed(egui::Key::Delete)
                            })
                        {
                            rich_layout.galley(
                                ui,
                                draft,
                                ui.available_width(),
                                &mention_users,
                                mentions::known_roles(state, channel),
                                &state.channels,
                                mass_mentions,
                                &mut self.avatars,
                                demo,
                            );
                            rich_layout.select_deleted_inline(ctx, composer_id);
                        }
                        let mut output = egui::ScrollArea::vertical()
                            .id_salt((composer_id, channel, editing_key))
                            .max_height(ui.ctx().viewport_rect().height() * 0.5)
                            .min_scrolled_height(ui.ctx().viewport_rect().height() * 0.5)
                            .auto_shrink([false, true])
                            .show(ui, |ui| {
                                let mut layouter = |ui: &egui::Ui, buffer: &dyn egui::TextBuffer, width: f32| {
                                    rich_layout.galley(
                                        ui,
                                        buffer.as_str(),
                                        width,
                                        &mention_users,
                                        mentions::known_roles(state, channel),
                                        &state.channels,
                                        mass_mentions,
                                        &mut self.avatars,
                                        demo,
                                    )
                                };
                                let output = TextEdit::multiline(draft)
                                    .interactive(keyboard_enabled)
                                    .layouter(&mut layouter)
                                    .id(composer_id)
                                    .event_filter(egui::EventFilter {
                                        horizontal_arrows: true, vertical_arrows: true, escape: editing_here,
                                        ..Default::default()
                                    })
                                    .char_limit(MAX_CONTENT)
                                    .desired_rows(1)
                                    .desired_width(f32::INFINITY)
                                    // Horizontal layouts reserve the interaction height, including around icons.
                                    .min_size(egui::vec2(0.0, ui.spacing().interact_size.y))
                                    .align(egui::Align2::LEFT_CENTER)
                                    .frame(egui::Frame::NONE)
                                    .hint_text(placeholder.as_str())
                                    .show(ui);
                                rich_layout.paint(ui, &output);
                                output
                            }).inner;
                        if !self.ime_active && !ime_this_frame {
                            rich_layout.snap_cursor(&mut output, ctx);
                        }
                        if mention_changed || (mention_enabled && suggestion_pointer) {
                            output.response.request_focus();
                        }
                        let mention_cursor = output
                            .cursor_range
                            .filter(|r| r.is_empty())
                            .map(|r| r.primary.index.0)
                            .or(cursor.filter(|_| suggestion_pointer))
                            .filter(|_| mention_enabled);
                        self.mention_menu
                            .refresh(state, channel, draft, mention_cursor, &mention_users);
                        if let Some(pick) = self.mention_menu.show(ui, composer_anchor, &mut self.avatars, demo)
                            && let Some(cursor) = mentions::insert(draft, pick)
                        {
                            output
                                .state
                                .cursor
                                .set_char_range(Some(egui::text::CCursorRange::one(
                                    egui::text::CCursor::new(cursor),
                                )));
                            output.state.clone().store(ctx, composer_id);
                            output.response.request_focus();
                            mention_changed = true;
                        }
                        #[cfg(not(feature = "demo"))]
                        let slash_preview = false;
                        #[cfg(feature = "demo")]
                        let slash_preview = state.demo && self.preview_slash_commands;
                        self.slash_commands.refresh(state, channel, draft,
                            !editing_here && keyboard_enabled && !self.ime_active && !ime_this_frame
                                && (output.response.has_focus() || suggestion_pointer || self.slash_commands.active.is_some() || slash_preview));
                        if let Some(pick) = self.slash_commands.show(ui, composer_anchor, state, channel, &mut self.avatars)
                            && let Some(cursor) = self.slash_commands.accept(pick, draft, remaining) {
                            output.state.cursor.set_char_range(Some(egui::text::CCursorRange::one(egui::text::CCursor::new(cursor))));
                            output.state.clone().store(ctx, composer_id);
                            output.response.request_focus();
                            mention_changed = true;
                        }
                        let edit = output.response;
                        let cleared = draft.is_empty();
                        if edit.changed() || mention_changed {
                            if editing_here && let Some((_, _, modified)) = &mut self.edit_modified {
                                *modified = true;
                            }
                            if editing_here {
                                self.edit_sent = false;
                            } else if cleared {
                                self.clear_draft(state, channel);
                            } else {
                                self.draft_changes.push(channel);
                            }
                        }
                        self.mention_lookup.text = self.mention_menu.member_query().to_owned();
                        self.mention_lookup.sync(ctx, state, channel, 0, commands);
                        if !self.mention_lookup.text.is_empty() && state.channel(channel).and_then(|c| c.guild).is_some() {
                            let search = &state.member_search[0];
                            if let Some(error) = search.error {
                                ui.small(error);
                                if ui.small_button("Retry member search").clicked() { self.mention_lookup.retry(); }
                            } else if !search.finished {
                                ui.small("Searching server members…");
                            } else if search.rows.is_empty() {
                                ui.small("No server matches. Try a username, nickname, or user ID.");
                            }
                        }
                        if !editing_here && (!new_draft.is_empty() || restore_empty_draft) {
                            state.drafts.insert(channel, new_draft);
                        }
                        edit.response
                            }).inner
                        }).inner;
                        if std::mem::take(&mut self.slash_commands.retry)
                            && let Some(command) = state.request_application_commands(channel, true) {
                            commands.push(command);
                        }
                        let slash_run = std::mem::take(&mut self.slash_commands.run)
                            && keyboard_enabled && !self.ime_active && !ime_this_frame
                            && self.slash_commands.can_submit(state, channel);
                        if (send || slash_run) && !cancel_edit {
                            if let Some((edit_channel, message, content)) = &editing {
                                if state.freshness == Freshness::Fresh
                                    && let Some(command) = state.prepare_edit(*edit_channel, *message, content.clone())
                                {
                                    commands.push(command);
                                    self.edit_sent = true;
                                } else {
                                    state.status = "Edit kept. Wait for your current message and connection, and enter nonempty text.";
                                }
                            } else if self.send_application_command(state, channel, commands)
                                || self.handle_builtin_slash(state, channel, ctx, commands) {
                            } else if !self.upload_busy && !(state.demo && self.attachment.is_some())
                                && let Some(command) = state.prepare_send_with_attachments(&self.selected_files().iter().map(|(name, _)| name.as_str()).collect::<Vec<_>>()) {
                                // Consume the selection in this UI pass, before desktop dispatch.
                                // A second render or Send gesture must not enqueue it again.
                                self.stage_pending_upload(ctx, &command);
                                self.timeline.follow_latest();
                                commands.push(command);
                            }
                            if !application_command { edit.request_focus(); }
                            if application_command && self.slash_commands.active.is_none() {
                                ctx.memory_mut(|memory| memory.request_focus(composer_id));
                            }
                        }
                    });
                });
                if !editing_here { self.slash_status(ui, state, channel); }
                if let Some((edit_channel, message)) = editing_key {
                    if state.freshness != Freshness::Fresh || !state.can_edit(edit_channel, message) { ui.weak("Editing this message is unavailable. Your text is kept until you cancel."); }
                } else if !state.can_send(channel) && !application_command {
                    ui.weak("Sending messages is unavailable in this conversation. Your draft is kept.");
                } else if self.attachment.is_some() && !state.can_attach(channel) {
                    ui.weak("Attaching files is unavailable here. Remove the attachment to send only text.");
                }
            });
		if editing_here {
			self.editing = if cancel_edit { None } else { editing };
			if cancel_edit {
				self.edit_sent = false;
			}
		}
		if self.edit_sent
			&& self.editing.as_ref().is_some_and(|(channel, id, content)| {
				state.timeline.get(*id).is_some_and(|message| {
					message.channel == *channel && message.content == *content
				})
			}) {
			self.editing = None;
			self.edit_sent = false;
		}
	}
	/// Selected-file cards above the composer input, in the style of Discord's upload tray.
	fn attachment_tray(&mut self, ui: &mut egui::Ui) {
		let colors = design::palette(ui);
		let textures = self.attachment_textures(ui.ctx());
		ui.add_space(4.0);
		let files = self.selected_files();
		egui::ScrollArea::horizontal()
			.id_salt("pending-attachments")
			.show(ui, |ui| {
				ui.horizontal(|ui| {
					ui.spacing_mut().item_spacing.x = 12.0;
					for (index, (filename, bytes)) in files.iter().enumerate() {
						ui.push_id(index, |ui| {
							if attachments::pending_card(
								ui,
								filename,
								*bytes,
								textures.get(index).and_then(Option::as_ref),
								!self.upload_busy,
							) {
								self.remove_attachment_index = Some(index);
							}
						});
					}
				});
			});
		ui.add_space(8.0);
		let line = ui.max_rect().x_range();
		let y = ui.cursor().top();
		ui.painter()
			.hline(line, y, egui::Stroke::new(1.0, colors.border));
		ui.add_space(6.0);
	}
	fn shows_title_bar(&self) -> bool {
		!cfg!(target_os = "linux") && !self.hide_title_bar
	}
	fn apply_pending_mention(
		&mut self,
		ctx: &egui::Context,
		composer_id: egui::Id,
		draft: &mut String,
		remaining: usize,
	) -> Option<MentionWrite> {
		let user_id = self.pending_mention.take()?;
		if user_id == Id(0) {
			return None;
		}
		let loaded = egui::text_edit::TextEditState::load(ctx, composer_id);
		let had_editor = loaded.is_some();
		let mut edit_state = loaded.unwrap_or_default();
		let range = edit_state
			.cursor
			.char_range()
			.filter(|_| had_editor)
			.or_else(|| {
				Some(egui::text::CCursorRange::one(egui::text::CCursor::new(
					draft.chars().count(),
				)))
			});
		let start = range.map_or(draft.chars().count(), |range| {
			range
				.as_sorted_char_range()
				.start
				.0
				.min(draft.chars().count())
		});
		let glue = start > 0
			&& draft
				.chars()
				.nth(start - 1)
				.is_some_and(|c| !c.is_whitespace());
		let token = mentions::user_mention_token(user_id);
		let token = if glue { format!(" {token}") } else { token };
		let Some(cursor) = emoji_picker::insert(draft, &token, range, remaining) else {
			return Some(MentionWrite::DidNotFit);
		};
		edit_state
			.cursor
			.set_char_range(Some(egui::text::CCursorRange::one(
				egui::text::CCursor::new(cursor),
			)));
		edit_state.store(ctx, composer_id);
		Some(MentionWrite::Inserted)
	}
	pub fn show(&mut self, ui: &mut egui::Ui, state: &mut State) -> Vec<Command> {
		if let Some(status) = state.take_user_action_status() {
			self.toasts.push(design::Level::Error, status);
		}
		let mention_waiting = self.pending_mention.is_some();
		let composer_open = self.editing.is_some()
			|| state.selected.is_some_and(|id| {
				state
					.channel(id)
					.is_some_and(|channel| channel.supports_text())
			});
		if mention_waiting && !composer_open {
			self.pending_mention = None;
			state.status = "This view has no message box, so the mention was not added.";
		}
		if self.editing.is_none()
			&& let Some(index) = state
				.message_actions
				.failed_edits
				.iter()
				.position(|(channel, _, _)| Some(*channel) == state.selected)
		{
			let (channel, message, content) = state.message_actions.failed_edits.remove(index);
			self.editing = Some((channel, message, content));
			self.edit_modified = Some((channel, message, true));
			self.edit_sent = false;
		}

		if self
			.pending_upload
			.as_ref()
			.is_some_and(|upload| !state.pending.iter().any(|p| p.nonce == upload.nonce))
		{
			self.pending_upload = None;
		}
		self.timeline.audio.seen = false;
		self.timeline.video.seen = false;
		let side = self.drain_side_press();
		let mut commands = Vec::new();
		if (side.back || side.forward)
			&& !self.timeline.video.is_fullscreen()
			&& !self.channel_menu.is_open()
		{
			if self.settings.open {
				if side.back {
					self.settings.open = false;
				}
			} else if self.server_settings.is_open() {
				if side.back {
					let _ = self.server_settings.navigate_away(state);
				}
			} else {
				let step = if side.back {
					state.navigate_back()
				} else {
					state.navigate_forward()
				};
				if let Some(NavStep {
					command: Some(command),
				}) = step
				{
					commands.push(command);
				}
			}
		}
		// Fullscreen playback owns the whole client surface, including during native resizing.
		if self.timeline.show_fullscreen_video(ui.ctx(), state) {
			ui.painter()
				.rect_filled(ui.max_rect(), 0, egui::Color32::BLACK);
			return commands;
		}
		self.profile_trigger = None;
		ui.ctx().data_mut(|data| {
			data.remove::<egui::Rect>(profiles::profile_opener_id());
		});
		let ctx = ui.ctx().clone();
		self.extensions.reset_theme_shortcut(&ctx);
		if self.extensions.has_result() {
			self.settings.open = false;
		}
		self.extensions
			.show_result(&ctx, state, &mut self.draft_changes, self.editing.is_some());
		self.keybinds_shortcut(&ctx);
		self.theme_preview_navigation(ui);
		let settings_open =
			self.settings.open || self.server_settings.is_open() || self.channel_menu.is_open();
		self.extensions.begin_theme_editor_frame();
		if self.settings.open {
			self.show_settings(&ctx, state, &mut commands);
			if self.extensions.previewing_theme() {
				self.settings.open = false;
			}
			ui.disable();
		}
		if self.server_settings.is_open() {
			self.extensions.stop_theme_preview(&ctx);
			if self.profile.is_some()
				&& ctx
					.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
			{
				self.profile = None;
				commands.push(state.clear_profile());
			}
			self.server_settings
				.show(&ctx, state, &mut self.avatars, &mut commands);
			if let Some(user) = self.server_settings.admin.profile.take() {
				self.profile = Some(user);
			}
			if self.profile.is_some() {
				ctx.set_sublayer(
					egui::LayerId::new(
						egui::Order::Foreground,
						egui::Id::unique("server-settings"),
					),
					egui::LayerId::new(
						egui::Order::Foreground,
						egui::Id::unique("user-profile-popout"),
					),
				);
			}
			if let Some(action) = self.server_settings.admin.user_action.take() {
				self.user_action = Some(action);
			}
			if let Some(channel) = self.server_settings.admin.message.take()
				&& self.server_settings.navigate_away(state)
				&& let Some(command) = state.select(channel)
			{
				commands.push(command);
			}
			ui.disable();
		}
		// Foreground confirmation handles Escape before background search/archive shortcuts.
		self.confirm_friend_removal(&ctx, state, &mut commands);
		if let Some(link) = self
			.timeline
			.opening
			.as_deref()
			.and_then(markdown::discord_chat_link)
		{
			self.timeline.opening = None;
			match state.open_chat_link(link.guild, link.channel, link.message) {
				Ok(Some(command)) => commands.push(command),
				Ok(None) => {}
				Err(message) => state.status = message,
			}
		}
		markdown::confirm_external_link(
			&ctx,
			&mut self.timeline.browser_opening,
			self.reading_preferences.confirm_external_links,
		);
		markdown::confirm_external_link(
			&ctx,
			&mut self.timeline.opening,
			self.reading_preferences.confirm_external_links,
		);
		let colors = crate::design::palette(ui);
		let background = crate::design::window_palette(ui);
		if !settings_open
			&& !self.switcher.is_open()
			&& !self.ime_active
			&& ctx.input(|input| {
				input.focused
					&& !input
						.events
						.iter()
						.any(|event| matches!(event, egui::Event::Ime(_)))
			}) && ctx.input_mut(|input| {
			crate::keybinds::pressed(
				input,
				self.keybinds
					.chord(model::KeybindAction::SwitchConversation),
			)
		}) {
			self.switcher.open(&ctx);
		}
		self.switcher_frame = self.switcher.is_open();
		if let Some(command) = state.select_opened_dm() {
			commands.push(command);
		}
		self.finish_slash_direct(state);
		if let Some(target) = self.switcher.show(&ctx, state, &mut self.avatars) {
			match target {
				switcher::Target::Channel(channel) => {
					if state.selected != Some(channel)
						&& let Some(command) = state.select(channel)
					{
						commands.push(command);
					}
					self.focus_switched_composer = state.selected == Some(channel)
						&& state
							.channel(channel)
							.is_some_and(|known| known.supports_text());
				}
				switcher::Target::Friend(user) => {
					if let Some(command) = state.open_friend_dm(user) {
						commands.push(command);
					}
				}
			}
		}
		if self.navigation_channel != state.selected {
			self.focus_switched_composer = state.selected.is_some();
			self.navigation_channel = state.selected;
			self.guild = state
				.selected
				.and_then(|id| state.channel(id))
				.and_then(|channel| channel.guild);
		}
		let title = self
			.guild
			.and_then(|id| state.guild(id))
			.map_or_else(|| "Direct Messages".to_owned(), |g| g.name.clone());
		if self.shows_title_bar() {
			self.title_bar(ui, state, &title);
		}
		// Server rail and channel list share one resizable column so the account card can
		// span both, like Discord's bottom-left user pill.
		let rail = notifications::RAIL_WIDTH;
		let sidebar_max = self.prepare_reading_sidebar(ui, "navigation", rail);
		let navigation = egui::Panel::left("navigation")
			.resizable(true)
			.show_separator_line(!design::has_window_background(ui))
			.default_size(rail + f32::from(self.reading_preferences.sidebar_width).min(sidebar_max))
			.size_range(rail + 190.0..=rail + sidebar_max)
			.frame(
				egui::Frame::new()
					.fill(if design::has_window_background(ui) {
						egui::Color32::TRANSPARENT
					} else {
						background.base
					})
					.inner_margin(0),
			)
			.show(ui, |ui| {
				egui::Panel::bottom("account-footer")
					.show_separator_line(false)
					.frame(
						egui::Frame::new()
							// The card's strip continues the channel list, so a window image
							// shows through it at the same opacity instead of full strength.
							.fill(if design::has_window_background(ui) {
								design::section_surface(
									ui,
									background.sidebar,
									design::ImageSection::ChannelList,
								)
							} else {
								egui::Color32::TRANSPARENT
							})
							.inner_margin(egui::Margin {
								left: 8,
								right: 8,
								top: 8,
								bottom: 8,
							}),
					)
					.show(ui, |ui| self.account_card(ui, state, &mut commands));
				self.notification_rail(ui, state, &mut commands);
				// The lists sit on their own rounded surface beside the rail, above the card.
				ui.painter().rect_filled(
					ui.available_rect_before_wrap(),
					if design::has_window_background(ui) {
						egui::CornerRadius::ZERO
					} else {
						egui::CornerRadius {
							nw: 8,
							sw: 8,
							..Default::default()
						}
					},
					design::section_surface(
						ui,
						background.sidebar,
						design::ImageSection::ChannelList,
					),
				);
				self.sidebar(ui, state, &title, &mut commands);
			});
		self.record_reading_sidebar(navigation.response.rect.width() - rail);
		if self.channel_preferences_changed {
			self.channel_cache.invalidate();
		}
		self.channel_menu
			.show(&ctx, state, self.guild, &mut self.avatars, &mut commands);
		if let Some((guild, channel)) = self.channel_menu.invite_requested.take() {
			self.server_menu.open_invite(state, guild, channel);
		}
		self.server_menu
			.show(&ctx, state, self.guild, &mut commands, &mut self.avatars);
		if let Some(guild) = self.server_menu.settings_requested.take()
			&& let Some(command) = self.preview_server_settings(state, guild)
		{
			commands.push(command);
		}
		if let Some(guild) = self.server_menu.mark_read_requested.take()
			&& let Some(command) = state.prepare_mark_guild_read(guild)
		{
			commands.push(command);
		}
		let selected_voice = state
			.selected
			.and_then(|id| state.channel(id))
			.is_some_and(|c| c.kind == 2);
		let selected_forum = state.selected.is_some_and(|id| state.is_forum(id));
		if let Some(id) = state.posting.created.take()
			&& let Some(command) = state.select(id)
		{
			commands.push(command);
		}
		let wide_members = ui.available_width() >= 720.0;
		self.search.sync(&ctx, state, &mut commands);
		let search_open =
			self.search.results_visible(state) && state.selected.is_some() && !selected_voice;
		let show_members = !selected_voice
			&& !search_open
			&& state.selected.is_some()
			&& if wide_members {
				self.reading_preferences.show_members
			} else {
				self.members_narrow_open
			};
		if search_open {
			self.channel_header(
				ui,
				state,
				selected_voice,
				show_members,
				wide_members,
				&mut commands,
			);
			let width = if wide_members {
				search::PANE_WIDTH.min(ui.available_width() * 0.45)
			} else {
				(ui.available_width() * 0.6).max(240.0)
			};
			egui::Panel::right("search-pane")
				.resizable(false)
				.show_separator_line(!design::has_window_background(ui))
				.exact_size(width)
				.frame(
					egui::Frame::new()
						.fill(design::section_surface(
							ui,
							background.sidebar,
							design::ImageSection::MemberList,
						))
						.inner_margin(egui::Margin::same(12)),
				)
				.show(ui, |ui| {
					self.search.pane(
						ui,
						state,
						&mut commands,
						&mut self.avatars,
						search::MediaUi {
							download: &mut self.timeline.download,
							audio: &mut self.timeline.audio,
							video: &mut self.timeline.video,
						},
					);
				});
		}
		if show_members {
			if state
				.members
				.as_ref()
				.is_none_or(|list| Some(list.channel) != state.selected)
				&& let Some(command) = state.request_members()
			{
				commands.push(command);
			}
			if wide_members {
				egui::Panel::right("people-pane")
					.resizable(false)
					.show_separator_line(!design::has_window_background(ui))
					.exact_size(240.0)
					.frame(
						egui::Frame::new()
							.fill(design::section_surface(
								ui,
								background.sidebar,
								design::ImageSection::MemberList,
							))
							.inner_margin(egui::Margin {
								left: 8,
								right: 8,
								top: 8,
								bottom: 8,
							}),
					)
					.show(ui, |ui| {
						self.member_rows(ui, state, &mut commands);
					});
			} else {
				let response = dialog::Dialog::new("members-narrow", "Members")
					.subtitle("Everyone with access to this conversation.")
					.width(360.0)
					.show(&ctx, |d| {
						let max = (d.available_height() - 180.0).clamp(120.0, 620.0);
						d.content(|ui| {
							ui.set_max_height(max);
							self.member_rows(ui, state, &mut commands);
						});
					});
				if response.close {
					self.members_narrow_open = false;
				}
			}
		} else if state.members.is_some() {
			commands.push(state.close_members());
		}
		self.reaction_picker.sync(state, state.selected);
		egui::CentralPanel::default()
			.frame(
				egui::Frame::new()
					.fill(if design::has_section_background(ui) {
						egui::Color32::TRANSPARENT
					} else {
						background.chat
					})
					.inner_margin(0),
			)
			.show(ui, |ui| {
				let message_surface =
					design::section_surface(ui, background.chat, design::ImageSection::MessageList);
				if design::has_section_background(ui)
					&& (state.selected.is_none() || selected_voice || selected_forum)
				{
					ui.painter().rect_filled(ui.max_rect(), 0, message_surface);
				}
				let warnings = state.startup_warnings;
				if !warnings.presence {
					self.friends.presence_warning_dismissed = None;
				}
				let unavailable: Vec<_> = [
					(warnings.read_state, "read status"),
					(warnings.notifications, "notification settings"),
					(warnings.sessions, "session status"),
					(warnings.emojis, "some server emoji"),
					(warnings.stickers, "some server stickers"),
				]
				.into_iter()
				.filter_map(|(unavailable, label)| unavailable.then_some(label))
				.collect();
				if !unavailable.is_empty() {
					egui::Frame::new()
						.inner_margin(egui::Margin::symmetric(16, 6))
						.show(ui, |ui| {
							ui.colored_label(
								colors.warning,
								format!(
									"Connected with some data unavailable: {}. Reconnect to retry.",
									unavailable.join(", ")
								),
							);
						});
				}
				if state.selected.is_none() && self.guild.is_none() {
					self.call_bar(ui, state, &mut commands);
					self.timeline.download.show_status(ui);
					self.friends_page(ui, state, &mut commands);
					return;
				}
				if !search_open {
					self.channel_header(
						ui,
						state,
						selected_voice,
						show_members,
						wide_members,
						&mut commands,
					);
				}
				let message_fill = (design::has_section_background(ui)
					&& state.selected.is_some()
					&& !selected_voice
					&& !selected_forum)
					.then(|| {
						(
							ui.painter().add(egui::Shape::Noop),
							ui.available_rect_before_wrap().top(),
						)
					});
				self.call_bar(ui, state, &mut commands);
				self.timeline.download.show_status(ui);
				let Some(channel) = state.selected else {
					ui.add_space((ui.available_height() * 0.32).max(24.0));
					ui.vertical_centered(|ui| {
						ui.label(
							design::semibold(ui, "No conversation selected", 20.0)
								.color(colors.text_strong),
						);
						ui.add_space(8.0);
						ui.label(
							RichText::new("Pick a channel or direct message from the list.")
								.color(colors.muted),
						);
					});
					return;
				};
				if selected_voice {
					if !self.voice_chat_open {
						self.voice_channel(ui, state, channel, &mut commands);
						return;
					}
					// Keep the existing conversation renderer beside the stage. On narrow
					// windows chat takes the body; Hide chat returns to the full stage.
					if ui.available_width() >= 720.0 {
						let chat_width = (ui.available_width() * 0.4).clamp(320.0, 440.0);
						egui::Panel::left("voice-stage")
							.exact_size(ui.available_width() - chat_width)
							.resizable(false)
							.frame(egui::Frame::NONE)
							.show(ui, |ui| {
								self.voice_channel(ui, state, channel, &mut commands);
							});
					}
				}
				if selected_forum {
					// A post's first message uses the same upload tray as the composer.
					let textures = self.attachment_textures(ui.ctx());
					let files = self.selected_files();
					let view = shortcuts::ShortcutView::new(
						&self.channel_preferences,
						self.shortcuts_available(state),
					);
					let mut staged = forum::Staged {
						files: &files,
						textures: &textures,
						choose: &mut self.attach_requested,
						remove: &mut self.remove_attachment_index,
						clear: &mut self.remove_attachment_requested,
						busy: self.upload_busy,
					};
					self.forum.show(
						ui,
						state,
						channel,
						&mut commands,
						(&mut self.scroll, &mut staged),
						(&mut self.channel_menu, view),
					);
					return;
				}
				egui::Panel::bottom("composer")
					.show_separator_line(false)
					.frame(
						egui::Frame::new()
							.fill(design::section_surface(
								ui,
								background.chat,
								design::ImageSection::Composer,
							))
							// The bottom inset matches the account card's, so the input and the
							// account pill sit on one line across the window.
							.inner_margin(egui::Margin {
								left: 16,
								right: 16,
								top: 2,
								bottom: 8,
							}),
					)
					.show(ui, |ui| {
						self.composer(ui, state, channel, &ctx, &mut commands);
					});
				if commands
					.iter()
					.any(|command| matches!(command, Command::Send { .. }))
					&& state.history_targeted
				{
					commands.push(state.history(None));
				}
				if let Some((shape, top)) = message_fill {
					let remaining = ui.available_rect_before_wrap();
					ui.painter().set(
						shape,
						egui::Shape::rect_filled(
							egui::Rect::from_min_max(
								egui::pos2(remaining.left(), top),
								remaining.right_bottom(),
							),
							0.0,
							message_surface,
						),
					);
				}
				let notices: Vec<String> = [
					match state.freshness {
						Freshness::Fresh | Freshness::Loading => None,
						Freshness::Stale => Some("Cached history · awaiting sync"),
						Freshness::Unavailable => Some("Conversation unavailable"),
					}
					.map(str::to_owned),
					state.read_state.status(channel).map(str::to_owned),
					(state.archived_thread.is_some() && state.archived_thread == state.selected)
						.then(|| "Opened from archive".to_owned()),
				]
				.into_iter()
				.flatten()
				.collect();
				if !notices.is_empty() {
					egui::Frame::new()
						.inner_margin(egui::Margin::symmetric(16, 4))
						.show(ui, |ui| {
							ui.label(
								RichText::new(notices.join(" · "))
									.size(11.0)
									.color(colors.muted),
							);
						});
				}
				egui::Frame::new()
					.inner_margin(egui::Margin {
						left: 0,
						right: 0,
						top: 4,
						bottom: 0,
					})
					.show(ui, |ui| {
						design::paint_chat_background(ui, ui.available_rect_before_wrap());
						self.timeline.hide_media_links = self.reading_preferences.hide_media_links;
						self.timeline.instant_scrolling =
							!self.reading_preferences.smooth_scrolling;
						self.timeline.extension_actions = self.extensions.message_actions();
						let mut seen = std::collections::BTreeSet::new();
						let author_lookup: Vec<_> = state
							.timeline
							.iter()
							.rev()
							.filter(|message| !message.author.webhook)
							.map(|message| message.author.id)
							.filter(|id| seen.insert(*id))
							.take(client_core::member_search::LIMIT)
							.collect();
						if let Some(command) = state.request_author_members(&author_lookup) {
							commands.push(command);
						}
						self.timeline.show_with_scroll(
							ui,
							state,
							&mut self.editing,
							&mut self.deleting,
							(&mut self.avatars, &mut self.profile),
							self.pending_upload.as_ref(),
							&mut self.scroll,
						);
						if let Some(id) = self.timeline.sticker_request.take() {
							if let Some(command) = state.request_sticker(id) {
								commands.push(command);
							}
							if let Some(command) = state.request_sticker_packs() {
								commands.push(command);
							}
						}
						if let Some(sticker) = self.timeline.browse_sticker.take() {
							self.emoji_picker.open_stickers(Some(&sticker));
						}
						if let Some(command) =
							state.request_author_members(&self.timeline.visible_authors)
						{
							commands.push(command);
						}
						if let Some((action, text)) = self.timeline.extension_request.take() {
							self.extensions
								.invoke_message(action, text, state, ui.ctx());
						}
						if let Some((message, anchor, trigger)) =
							self.timeline.reaction_picker.take()
						{
							self.emoji_picker.dismiss(state, &mut commands);
							self.reaction_picker
								.open_reaction(state, message, anchor, trigger);
						}
						self.reaction_picker.show_reaction(
							ui,
							state,
							&mut self.avatars,
							&mut commands,
						);
						if let Some((message, emoji, open)) = self.timeline.reaction_users.take()
							&& let Some(command) =
								state.request_reaction_users(message, emoji, open)
						{
							commands.push(command);
						}
						crate::reactions::show_users(
							ui.ctx(),
							state,
							&mut self.avatars,
							&mut commands,
						);
						if let Some((channel, message)) = self.timeline.quick_delete.take()
							&& let Some(command) = state.prepare_delete(channel, message)
						{
							commands.push(command);
						}
						if let Some(id) = self.timeline.remove_preserved.take() {
							state.discard_preserved_deleted(id);
						}
						if let Some(nonce) = self.timeline.restore_pending.take() {
							if state
								.pending
								.iter()
								.any(|p| p.nonce == nonce && p.sticker.is_some())
							{
								state.discard_pending_sticker(&nonce);
							} else {
								self.restore_pending(state, channel, &nonce);
							}
						}
						self.cancel_upload_requested |=
							std::mem::take(&mut self.timeline.cancel_upload);
						if let Some(gif) = self.timeline.gif_favorite.take() {
							state.toggle_gif_favorite(&gif);
						}
						match self.timeline.invite_action.take() {
							Some(invites::Action::OpenGuild(guild))
								if state.guild(guild).is_some() =>
							{
								self.guild = Some(guild);
								if let Some(command) = state.select_guild(guild) {
									commands.push(command);
								}
							}
							Some(invites::Action::Join(code)) => {
								if state.invite_join.code == code
									&& state.invite_challenge().is_some()
								{
									self.verification.active = false;
								} else if let Some(command) = state.join_invite(code) {
									commands.push(command);
								}
							}
							_ => {}
						}
						for code in std::mem::take(&mut self.timeline.invite_requests) {
							if let Some(command) = state.request_invite(code) {
								commands.push(command);
							}
						}
						if std::mem::take(&mut self.timeline.edit_started) {
							self.edit_modified = None;
							self.edit_undo_cleared = false;
							self.composer_edit = None;
							self.edit_sent = false;
						}
						if std::mem::take(&mut self.timeline.reply_started) {
							self.focus_switched_composer = true;
							ctx.request_repaint();
						}
						if let Some(target) = self.timeline.reply_target.take() {
							if let Some(command) = state.open_reply_target(target) {
								commands.push(command);
							}
							ctx.request_repaint();
						} else if std::mem::take(&mut self.timeline.unread_jump) {
							if let Some(command) = state.open_unread() {
								commands.push(command);
							}
							ctx.request_repaint();
						} else if std::mem::take(&mut self.timeline.load_newer) {
							if let Some(command) = state.newer_history() {
								commands.push(command);
							}
							ctx.request_repaint();
						} else if std::mem::take(&mut self.timeline.latest) {
							commands.push(state.history(None));
						} else if std::mem::take(&mut self.timeline.load_older)
							&& let Some(command) = state.older_history()
						{
							commands.push(command);
						}
					});
			});
		if state
			.archives
			.as_ref()
			.is_some_and(|view| self.guild != Some(view.guild))
		{
			commands.push(state.clear_archives());
		}
		if let Some((channel, message, pinned)) = self.timeline.pin_request.take()
			&& let Some(command) = state.prepare_pin(channel, message, pinned)
		{
			commands.push(command);
		}
		if let Some(channel) = state.pins_changed.take()
			&& self.search.open
			&& self.search.pins()
			&& state.selected == Some(channel)
			&& let Some(command) = state.request_pins()
		{
			commands.push(command);
		}
		self.search
			.overlays(&ctx, state, &mut self.avatars, &mut commands);
		if state.invite_challenge().is_none() {
			self.join_server
				.show(&ctx, state, &mut self.avatars, &mut commands);
		}
		if let Some(anchor) = self.pins_anchor {
			let dm = state
				.channels
				.iter()
				.any(|c| Some(c.id) == state.selected && c.guild.is_none());
			self.search.pins_popout(
				ui,
				state,
				anchor,
				dm,
				&mut commands,
				&mut self.avatars,
				search::MediaUi {
					download: &mut self.timeline.download,
					audio: &mut self.timeline.audio,
					video: &mut self.timeline.video,
				},
			);
			if !(self.search.open && self.search.pins()) {
				self.pins_anchor = None;
			}
		}
		if let Some(link) = self.search.opening.take() {
			self.timeline.opening = Some(link);
		}
		if let Some(profile) = self.search.profile.take() {
			self.profile = Some(profile);
		}
		if let Some(channel) = self.search.channel_reference.take() {
			self.timeline.channel_reference = Some(channel);
		}
		// A forum pane lists its own archived posts inline instead of the floating window.
		if !(selected_forum
			&& state
				.archives
				.as_ref()
				.is_some_and(|view| Some(view.parent) == state.selected))
		{
			self.archives
				.show(&ctx, state, &mut commands, &mut self.avatars);
		}
		if let Some(parent) = self.timeline.threads_request.take() {
			self.archive_parent = Some(parent);
		}
		if let Some((channel, message)) = self.timeline.thread_request.take() {
			state.clear_channel_action_result(channel);
			// Discord seeds the name from the starter's first line; the user can still edit it.
			let name = state
				.timeline
				.get(message)
				.map(|m| thread_create::suggested_name(&m.display_text()))
				.unwrap_or_default();
			self.thread_create.open(channel, Some(message), name);
		}
		// A thread shows the message it hangs off; read it lazily like other per-channel data.
		if let Some(command) = state.request_thread_starter() {
			commands.push(command);
		}
		if let Some(channel) = self.archives.create_requested.take() {
			// The editor replaces the Threads dialog instead of stacking on it.
			commands.push(state.clear_archives());
			state.clear_channel_action_result(channel);
			self.thread_create.open(channel, None, String::new());
		}
		self.thread_create.show(&ctx, state, &mut commands);
		self.screen.show(&ctx, state);
		if let Some(id) = self.timeline.channel_reference.take() {
			state.clear_channel_action_result(id);
			self.timeline.pending_channel_reference = Some(id);
		}
		if let Some(id) = self.timeline.pending_channel_reference {
			if let Some(target) = state
				.channel(id)
				.filter(|c| c.guild.is_some() && (c.supports_text() || matches!(c.kind, 15 | 16)))
			{
				self.timeline.pending_channel_reference = None;
				self.guild = target.guild;
				if let Some(command) = state.select(id) {
					commands.push(command);
				}
			} else if state.channel_action_status(id).is_some() {
				self.timeline.pending_channel_reference = None;
			} else if let Some(guild) = state
				.selected
				.and_then(|selected| state.channel(selected))
				.and_then(|source| source.guild)
			{
				if let Some(command) = state.request_channel_reference(guild, id) {
					commands.push(command);
				}
			} else {
				self.timeline.pending_channel_reference = None;
			}
		}
		if let Some(message) = self.timeline.mark_unread.take() {
			self.timeline.mark_read = None;
			if !settings_open && let Some(command) = state.prepare_mark_unread(message) {
				self.timeline.browse_away();
				commands.push(command);
			}
		} else if let Some(message) = self.timeline.mark_read.take()
			&& !settings_open
			&& state.search_target.is_none()
			&& !state.history_targeted
			&& let Some(command) = state.prepare_mark_read(message)
		{
			commands.push(command);
		}

		if let Some(message) = self.timeline.forward_request.take() {
			self.forwarding.open(state, message);
		}
		self.forwarding
			.show(&ctx, state, &mut self.avatars, &mut commands);

		if let Some((message, custom_id, values)) = self.timeline.component_action.take()
			&& let Some(command) = state.prepare_component(message, &custom_id, values)
		{
			commands.push(command);
		}
		if let Some(channel) = state.selected {
			if state.interactions.modal.is_none()
				&& self.timeline.components.remote_search
				&& !self.timeline.components.search.text.is_empty()
			{
				self.timeline
					.components
					.search
					.sync(&ctx, state, channel, 0, &mut commands);
			}
			if self.interaction_components.remote_search
				&& !self.interaction_components.search.text.is_empty()
			{
				self.interaction_components
					.search
					.sync(&ctx, state, channel, 0, &mut commands);
			}
		}
		self.interaction_components.dialogs(
			&ctx,
			state,
			&mut commands,
			&mut self.avatars,
			&mut self.timeline.opening,
		);
		self.interaction_file_request = self.interaction_components.file_request.take();
		if !state.interactions.ephemeral.is_empty() {
			let messages = state.interactions.ephemeral.clone();
			egui::Window::new("Only you can see these responses")
				.id(egui::Id::unique("interaction-responses"))
				.show(&ctx, |ui| {
					egui::ScrollArea::vertical()
						.max_height(400.0)
						.show(ui, |ui| {
							for message in &messages {
								ui.strong(&message.author.name);
								self.interaction_components.show_text(
									ui,
									message,
									state,
									&mut self.avatars,
									&mut self.timeline.opening,
								);
								if !message.extra_content.components_v2
									&& self.interaction_components.media_visible(
										ui,
										message,
										state.generation,
									) {
									if embeds::has_media_spoilers(message) {
										self.timeline.reveal_private_media(message);
									}
									if let Some(gif) = embeds::show(
										ui,
										message,
										&mut self.profile_formatted,
										&mut self.avatars,
										&mut self.timeline.opening,
										&mut self.timeline.download,
										&mut self.profile,
										state,
									) {
										self.timeline.gif_favorite = Some(gif);
									}
									let mut surface =
										select::Surface::new(ui, ("ephemeral-media", message.id));
									attachments::show(
										ui,
										message,
										&mut self.avatars,
										&mut self.timeline.viewing,
										&mut self.timeline.opening,
										&mut self.timeline.download,
										&mut self.timeline.audio,
										&mut self.timeline.video,
										state.demo,
										&mut surface,
									);
									surface.finish(ui);
								}
								if let Some((id, custom_id, values)) =
									self.interaction_components.show(
										ui,
										message,
										state,
										&mut self.avatars,
										&mut self.timeline.opening,
										&mut components::MediaUi {
											component_viewing: &mut self.timeline.component_viewing,
											viewing: &mut self.timeline.viewing,
											download: &mut self.timeline.download,
											audio: &mut self.timeline.audio,
											video: &mut self.timeline.video,
										},
									) && let Some(command) =
									state.prepare_component(id, &custom_id, values)
								{
									commands.push(command);
								}
								if ui.small_button("Dismiss").clicked() {
									state.dismiss_ephemeral(message.id);
									self.interaction_components.forget_message(message.id);
								}
								ui.separator();
							}
						});
				});
		}
		if let Some((message, emoji)) = self.timeline.reaction.take() {
			if let Some(emoji) = emoji {
				if let Some(command) = state.prepare_reaction(message, emoji) {
					commands.push(command);
				}
			} else {
				state.refresh_reactions(message);
			}
		}
		self.group_menu
			.show(&ctx, state, &mut self.avatars, &mut commands);
		for intent in self
			.channel_menu
			.shortcut_requested
			.take()
			.into_iter()
			.chain(self.group_menu.pin_requested.take())
		{
			self.apply_shortcut(state, intent);
		}
		if let Some(action) = self.user_action.take().or(self.timeline.user_action.take()) {
			let command = match action {
				user_menu::Action::Note(user) => {
					self.profile = None;
					self.contact_editor.open(user, false, state)
				}
				user_menu::Action::Nickname(user) => {
					self.profile = None;
					self.contact_editor.open(user, true, state)
				}
				user_menu::Action::Shortcut(intent) => {
					self.apply_shortcut(state, intent);
					None
				}
				user_menu::Action::Mention(user) => {
					if user.id != Id(0) {
						self.pending_mention = Some(user.id);
						ctx.request_repaint();
					}
					None
				}
				action => user_menu::prepare(action, state),
			};
			if let Some(command) = command {
				commands.push(command);
			}
		}
		self.contact_editor.show(&ctx, state, &mut commands);
		if let Some(user) = &self.profile {
			let profile_guild = self.server_settings.guild().or_else(|| {
				state
					.channels
					.iter()
					.find(|channel| Some(channel.id) == state.selected)
					.and_then(|channel| channel.guild)
			});
			if user.webhook {
				if state.profile.is_some() {
					commands.push(state.clear_profile());
				}
			} else if state
				.profile
				.as_ref()
				.is_none_or(|p| p.user != user.id || p.guild != profile_guild)
			{
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
			let anchor = match self.profile_anchor {
				Some((id, pos)) if id == user.id => pos,
				_ => {
					let pos = ctx
						.input(|i| i.pointer.interact_pos().or(i.pointer.latest_pos()))
						.unwrap_or_else(|| ctx.content_rect().center());
					self.profile_anchor = Some((user.id, pos));
					pos
				}
			};
			if let Some(rect) =
				ctx.data(|data| data.get_temp::<egui::Rect>(profiles::profile_opener_id()))
			{
				self.profile_trigger = Some(rect);
			}
			match profiles::show(
				ui,
				user,
				state.profile.as_ref(),
				state,
				&mut self.avatars,
				&mut self.profile_link,
				&mut self.profile_formatted,
				self.reading_preferences.confirm_external_links,
				anchor,
			) {
				Some(profiles::Action::Avatar(media)) => {
					self.profile_image = Some((
						state.generation,
						model::Attachment {
							id: Id(0),
							filename: "avatar.png".into(),
							description: Some(format!("{}’s profile picture", user.name)),
							content_type: Some("image/png".into()),
							size: 0,
							media,
							spoiler: false,
							duration_ms: None,
							waveform: Vec::new(),
						},
					));
					self.profile = None;
					self.profile_link = None;
					self.profile_anchor = None;
					commands.push(state.clear_profile());
				}
				Some(profiles::Action::AddFriend(id)) => {
					if let Some(command) = state.add_profile_friend(id) {
						commands.push(command);
					}
				}
				Some(profiles::Action::AcceptFriend(id)) => {
					if let Some(command) = state.resolve_friend_request(id, true) {
						commands.push(command);
					}
				}
				Some(profiles::Action::RemoveFriend) => {
					self.friend_removal = Some((state.generation, user.clone()));
				}
				Some(profiles::Action::Menu(action)) => {
					// Dispatched with the other user menus on the next frame.
					self.user_action = Some(action);
				}
				Some(profiles::Action::Edit) => {
					self.profile = None;
					self.profile_link = None;
					self.profile_anchor = None;
					commands.push(state.clear_profile());
					self.preview_settings("profile");
				}
				Some(profiles::Action::Profile(user)) => {
					self.profile = Some(user);
					self.profile_link = None;
					commands.push(state.clear_profile());
				}
				Some(profiles::Action::Close) => {
					let keep = self.profile_trigger.is_some_and(|rect| {
						ctx.input(|input| {
							input.pointer.any_pressed()
								&& input
									.pointer
									.interact_pos()
									.is_some_and(|pos| rect.contains(pos))
						})
					});
					if !keep {
						self.profile = None;
						self.profile_link = None;
						self.profile_anchor = None;
						commands.push(state.clear_profile());
					}
				}
				Some(profiles::Action::Retry) => {
					if let Some(command) = state.request_profile(user.id, profile_guild) {
						commands.push(command);
					}
				}
				Some(profiles::Action::Message(channel)) => {
					self.profile = None;
					self.profile_link = None;
					self.profile_anchor = None;
					commands.push(state.clear_profile());
					if self.server_settings.navigate_away(state)
						&& let Some(command) = state.select(channel)
					{
						commands.push(command);
					}
				}
				None => {}
			}
		} else {
			self.profile_anchor = None;
			self.profile_link = None;
		}

		if let Some((generation, image)) = &self.profile_image
			&& (*generation != state.generation
				|| attachments::viewer(
					ui,
					std::slice::from_ref(image),
					image.id,
					&mut self.avatars,
					&mut self.timeline.download,
					&mut self.timeline.opening,
					state.demo,
				)
				.is_none())
		{
			self.profile_image = None;
		}

		self.reconcile_edit(state);
		if self.deleting.is_some_and(|(channel, message)| {
			state.selected != Some(channel) || state.timeline.get(message).is_none()
		}) {
			self.deleting = None;
		}
		if let Some((channel, message)) = self.deleting {
			let allowed = state.can_delete(channel, message);
			let conversation = state
				.channel(channel)
				.map_or("this conversation", |c| c.name.as_str());
			let mut confirm = dialog::Confirm::new(
				"delete-message",
				"Delete message?",
				format!(
					"This permanently removes the selected message from {conversation} for everyone."
				),
			)
			.danger()
			.confirm_label("Delete")
			.cancel_label("Keep Message")
			.enabled(allowed);
			if !allowed {
				confirm = confirm.note(
					dialog::Level::Warning,
					"Deleting this message is unavailable.",
				);
			}
			match confirm.show(&ctx) {
				Some(dialog::Choice::Confirmed) => {
					if let Some(command) = state.prepare_delete(channel, message) {
						commands.push(command);
					}
					self.deleting = None;
				}
				Some(dialog::Choice::Cancelled) => self.deleting = None,
				None => {}
			}
		}
		self.show_call_switch(&ctx, state, &mut commands);
		self.verification.show(&ctx, state);
		self.scroll.clear_if_unbound(&ctx);
		self.scroll.paint(&ctx);
		// Clear the title bar and channel header so a notice never sits on the chrome.
		self.toasts
			.show(&ctx, if self.shows_title_bar() { 96.0 } else { 60.0 });
		if !commands.is_empty() {
			ctx.request_repaint();
		}
		commands
	}
}

#[cfg(test)]
mod composer_tests {
	use super::*;

	#[test]
	fn image_surface_reaches_the_channel_header_without_a_stripe() {
		let ctx = egui::Context::default();
		ctx.set_theme(egui::ThemePreference::Dark);
		let mut theme = extensions::Theme::default();
		theme.dark.background = Some(extensions::Background {
			sections: Some(extensions::SectionOpacity::default()),
			..Default::default()
		});
		design::set_extension_theme(Some(&theme));
		design::set_background_image(
			&ctx,
			Some(std::sync::Arc::new(egui::ColorImage::filled(
				[1, 1],
				egui::Color32::WHITE,
			))),
		);
		let mut state = test_support::demo_state();
		let mut view = MessagingUi::default();
		let mut output = ctx.run_ui(
			egui::RawInput {
				screen_rect: Some(egui::Rect::from_min_size(
					egui::Pos2::ZERO,
					egui::vec2(1100.0, 800.0),
				)),
				..Default::default()
			},
			|ui| {
				view.show(ui, &mut state);
			},
		);
		let message_alpha = (75 * 255 / 100) as u8;
		let expected_top = if view.shows_title_bar() { 84.0 } else { 48.0 };
		let surface_reaches_header = output.shapes.iter().any(|shape| match &shape.shape {
			egui::Shape::Rect(rect)
				if rect.fill.a() == message_alpha && rect.rect.height() > 200.0 =>
			{
				(rect.rect.top() - expected_top).abs() <= 1.0
			}
			_ => false,
		});
		output.textures_delta.clear();
		design::set_extension_theme(None);
		assert!(
			surface_reaches_header,
			"message surface did not reach expected top {expected_top}",
		);
	}

	#[test]
	fn composer_placeholder_alignment() {
		for scale in [1.0, 1.25, 1.5, 2.0] {
			for theme in [egui::Theme::Dark, egui::Theme::Light] {
				let ctx = egui::Context::default();
				fonts::install(&ctx);
				design::apply(&ctx);
				ctx.set_theme(theme);
				ctx.set_pixels_per_point(scale);
				let mut state = edit_state();
				state.channels[0].name = "Alex".into();
				let mut view = MessagingUi::default();
				for width in [320.0, 900.0] {
					for draft in ["", "Message @Alex", "First line\nSecond line"] {
						state.drafts.insert(Id(10), draft.into());
						for _ in 0..2 {
							let mut empty_height = 0.0;
							let output = ctx.run_ui(
								egui::RawInput {
									screen_rect: Some(egui::Rect::from_min_size(
										egui::Pos2::ZERO,
										egui::vec2(width, 300.0),
									)),
									..Default::default()
								},
								|ui| {
									view.composer(ui, &mut state, Id(10), &ctx, &mut vec![]);
									empty_height = view
										.composer_layout
										.galley(
											ui,
											"",
											300.0,
											&[],
											&[],
											&[],
											false,
											&mut view.avatars,
											true,
										)
										.rect
										.height();
								},
							);
							let frame = output
								.shapes
								.iter()
								.find_map(|s| match &s.shape {
									egui::Shape::Rect(r) => Some(r.rect),
									_ => None,
								})
								.unwrap();
							let text = output
								.shapes
								.iter()
								.find_map(|s| match &s.shape {
									egui::Shape::Text(t)
										if t.galley.job.text == draft
											|| t.galley.job.text == "Message @Alex" =>
									{
										Some(t.galley.rect.translate(t.pos.to_vec2()))
									}
									_ => None,
								})
								.unwrap();
							output.drop_without_applying_deltas();
							assert!(
								(text.center().y - frame.center().y).abs() <= 1.0 / scale,
								"{draft:?}, scale {scale}, width {width}: text {text:?}, frame {frame:?}"
							);
							assert!(
								empty_height >= 15.0,
								"empty editor must retain a full-height caret"
							);
						}
					}
				}
			}
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

	fn edit_state() -> State {
		let user = model::User {
			id: Id(1),
			name: "Alex".into(),
			avatar: None,
			webhook: false,
			kind: Default::default(),
			discriminator: 0,
			primary_guild: None,
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
					sticker_items: vec![],
					id: Id(20),
					channel: Id(10),
					author: user,
					content: "Original".into(),
					prior_contents: Default::default(),
					edited: false,
					edited_at: None,
					revision: 0,
					nonce: None,
					reply_to: None,
					kind: 0,
					reply_deleted: false,
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
			assert_eq!(state.selected, Some(Id(11)));
			assert_eq!(view.guild, Some(Id(100)));
			assert_eq!(state.search_target, target_message);
			assert_eq!(state.drafts, drafts);
			assert_eq!(commands.iter().filter(|command| matches!(command, Command::History { channel: Id(11), before, .. } if *before == target_message.map(|id| Id(id.0 + 1)))).count(), 1);
			assert!(!commands.iter().any(|command| matches!(
				command,
				Command::Send { .. } | Command::Edit { .. } | Command::Delete { .. }
			)));
		}
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
		assert!(!view.has_edit(), "accepted edits close immediately");
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
		assert!(!view.has_edit());
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
		fn collect(shape: &egui::Shape, labels: &mut Vec<(String, egui::Rect)>) {
			match shape {
				egui::Shape::Text(text) => labels.push((
					text.galley.job.text.clone(),
					text.galley.rect.translate(text.pos.to_vec2()),
				)),
				egui::Shape::Vec(shapes) => {
					for shape in shapes {
						collect(shape, labels);
					}
				}
				_ => {}
			}
		}
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
				assert!(output.platform_output.commands.is_empty());
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
					let commands = click(&mut view, &mut state, "Next messages");
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
	fn reply_controls_are_inert_when_deleted_loading_stale_or_unavailable() {
		fn collect(shape: &egui::Shape, labels: &mut Vec<(String, egui::Rect)>) {
			match shape {
				egui::Shape::Text(text) => labels.push((
					text.galley.job.text.clone(),
					text.galley.rect.translate(text.pos.to_vec2()),
				)),
				egui::Shape::Vec(shapes) => {
					for shape in shapes {
						collect(shape, labels);
					}
				}
				_ => {}
			}
		}
		for blocked in 0..5 {
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
				assert!(output.platform_output.commands.is_empty());
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
		fn collect(shape: &egui::Shape, labels: &mut Vec<(String, egui::Rect)>) {
			match shape {
				egui::Shape::Text(text) => labels.push((
					text.galley.job.text.clone(),
					text.galley.rect.translate(text.pos.to_vec2()),
				)),
				egui::Shape::Vec(shapes) => {
					for shape in shapes {
						collect(shape, labels);
					}
				}
				_ => {}
			}
		}
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
					|ui| commands = view.show(ui, state),
				);
				assert!(output.platform_output.commands.is_empty());
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
			assert_eq!(state.search_target, Some(Id(19)));
			if loaded {
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
				assert_eq!(
					commands
						.iter()
						.filter(|command| matches!(
							command,
							Command::History {
								channel: Id(10),
								before: Some(Id(20)),
								..
							}
						))
						.count(),
					1
				);
				assert!(state.history_pending);
				assert_eq!(state.freshness, Freshness::Loading);
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
					if let Some((_, rect)) =
						labels.iter().find(|(text, _)| text == "Reload history")
					{
						reload = Some(rect.center());
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
			}
		}
	}

	#[test]
	fn deleted_edit_target_keeps_user_changes_but_releases_untouched_original() {
		fn collect(shape: &egui::Shape, labels: &mut Vec<(String, egui::Rect)>) {
			match shape {
				egui::Shape::Text(text) => labels.push((
					text.galley.job.text.clone(),
					text.galley.rect.translate(text.pos.to_vec2()),
				)),
				egui::Shape::Vec(shapes) => {
					for shape in shapes {
						collect(shape, labels);
					}
				}
				_ => {}
			}
		}
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
				assert!(output.platform_output.commands.is_empty());
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
					if labels.iter().any(|(text, _)| text == "Edit message") {
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
	fn member_badges_and_subtitles_stay_inside_adjacent_rows() {
		for light in [false, true] {
			for width in [180.0, 240.0] {
				let ctx = egui::Context::default();
				ctx.set_theme(if light {
					egui::ThemePreference::Light
				} else {
					egui::ThemePreference::Dark
				});
				design::apply(&ctx);
				let mut state = State {
					demo: true,
					selected: Some(Id(1)),
					members: Some(model::MemberList {
						channel: Id(1),
						guild: Some(Id(2)),
						request: 1,
						total: 3,
						lazy: false,
						groups: vec![],
						ranges: vec![],
						freshness: Freshness::Fresh,
						start: 0,
						slots: (1..=3)
							.map(|id| {
								Some(model::MemberSlot::Person(model::Member {
									user: model::User {
										id: Id(id),
										name: format!("Member {id}"),
										avatar: None,
										kind: model::AccountKind::Bot,
										webhook: false,
										discriminator: 0,
										primary_guild: (id == 1).then(|| {
											Box::new(model::ClanTag {
												guild: Id(9),
												tag: "SPDY".into(),
												badge: None,
											})
										}),
									},
									nick: None,
									roles: vec![],
									status: Some("online".into()),
									custom_status: Some(format!("Activity {id}")),
									activities: vec![],
								}))
							})
							.collect(),
					}),
					..Default::default()
				};
				let mut messaging = MessagingUi::default();
				for hover in [false, true] {
					let mut origin = egui::Pos2::ZERO;
					let mut output = ctx.run_ui(
						egui::RawInput {
							screen_rect: Some(egui::Rect::from_min_size(
								egui::Pos2::ZERO,
								egui::vec2(width, 260.0),
							)),
							events: if hover {
								vec![egui::Event::PointerMoved(egui::pos2(80.0, 105.0))]
							} else {
								vec![]
							},
							..Default::default()
						},
						|ui| {
							origin = ui.cursor().min;
							let mut commands = vec![];
							messaging.member_rows(ui, &mut state, &mut commands);
						},
					);
					output.textures_delta.clear();
					for id in 1..=3 {
						let row = egui::Rect::from_min_size(
							origin + egui::vec2(0.0, id as f32 * 42.0),
							egui::vec2(width, 42.0),
						);
						let mut labels = vec![format!("Member {id}"), format!("Activity {id}")];
						if id == 1 {
							labels.push("SPDY".into());
						}
						for label in labels {
							let text = output
								.shapes
								.iter()
								.find_map(|s| match &s.shape {
									egui::Shape::Text(t) if t.galley.job.text == label => {
										Some(egui::Rect::from_min_size(t.pos, t.galley.size()))
									}
									_ => None,
								})
								.expect("visible member text");
							assert!(
								text.top() >= row.top() && text.bottom() <= row.bottom() - 1.0,
								"{label}, hover={hover}: text {text:?} exceeds row {row:?}"
							);
						}
					}
					assert!(messaging.take_avatar_requests().is_empty());
					output.drop_without_applying_deltas();
				}
			}
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
				message_count: None,
				icon: None,
			}],
			members: Some(model::MemberList {
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
								id: Id(id),
								name: format!("Synthetic {id}"),
								avatar: None,
								webhook: false,
								kind: Default::default(),
								discriminator: 0,
								primary_guild: None,
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
					id: Id(8),
					bits: 0,
					name: "Founders".into(),
					color: 0xe78284,
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
		let channel = state.selected.unwrap();
		let guild = state
			.channels
			.iter()
			.find(|entry| entry.id == channel)
			.unwrap()
			.guild
			.unwrap();
		let user = test_support::message(499, channel).author;
		// No read acknowledgement is part of this presence-only scenario: clear the page and
		// the latest-message metadata that an empty live edge would otherwise acknowledge.
		state.timeline.clear();
		if let Some(channel) = state.channels.iter_mut().find(|c| c.id == channel) {
			channel.last_message = None;
		}
		state.members = Some(model::MemberList {
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
		state
			.drafts
			.insert(channel, "Keep this unsent draft".into());
		let mut messaging = MessagingUi {
			navigation_channel: Some(channel),
			guild: Some(guild),
			profile: Some(user.clone()),
			profile_anchor: Some((user.id, egui::pos2(420.0, 150.0))),
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
			assert_eq!(messaging.profile.as_ref().unwrap().id, user.id);
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
				}))],
			});
			let mut messaging = MessagingUi {
				navigation_channel: Some(channel),
				guild,
				profile: Some(user.clone()),
				profile_anchor: Some((user.id, egui::pos2(420.0, 150.0))),
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
						}],
					}
				} else {
					client_core::Event::DirectPresence(vec![client_core::presence::Update {
						user: user.id,
						status: model::Patch::Value("online".into()),
						activities: model::Patch::Value(activities),
						custom_status: model::Patch::Absent,
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
					assert!(output.platform_output.commands.is_empty());
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
				profile: Some(user),
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
		fn labels(shape: &egui::Shape, out: &mut Vec<(String, egui::Rect)>) {
			match shape {
				egui::Shape::Text(text) => out.push((
					text.galley.job.text.clone(),
					text.galley.rect.translate(text.pos.to_vec2()),
				)),
				egui::Shape::Vec(shapes) => shapes.iter().for_each(|shape| labels(shape, out)),
				_ => {}
			}
		}
		let ctx = egui::Context::default();
		let mut state = test_support::demo_state();
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
		assert_eq!(messaging.profile.as_ref().map(|user| user.id), Some(Id(2)));
		click(&mut messaging, &mut state, pos);
		assert!(messaging.profile.is_none());
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
				id: Id(2),
				name: "Synthetic".into(),
				avatar: None,
				webhook: false,
				kind: Default::default(),
				discriminator: 0,
				primary_guild: None,
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
					message_count: None,
					icon: None,
				}],
				..Default::default()
			};
			let mut messaging = MessagingUi {
				navigation_channel: state.selected,
				guild: Some(Id(99)),
				profile: Some(user),
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
					icons::button(ui, icons::Icon::Forward, 28.0, "Forward message");
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
