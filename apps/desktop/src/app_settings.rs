use local_store::AppPreferences;

pub const MIN_WINDOW_SIZE: [u32; 2] = [760, 520];

#[derive(Default)]
pub struct Settings {
	pub current: AppPreferences,
	pub loaded: bool,
	pub state: crate::toggle_setting::Settings,
}

impl Settings {
	pub fn from_preferences(value: Result<AppPreferences, local_store::StoreError>) -> Self {
		let mut settings = Self {
			loaded: value.is_ok(),
			current: value.unwrap_or_default(),
			..Self::default()
		};
		settings.state.failed = !settings.loaded;
		settings
	}
	pub fn save(&mut self, cache: Option<&crate::cache::Cache>, generation: u64) -> bool {
		// Never replace an unread preference row with startup defaults after a read failure.
		if !self.loaded {
			self.state.failed = true;
			return false;
		}
		if !self.state.dirty || self.state.saving {
			return false;
		}
		let accepted = cache.is_some_and(|cache| {
			cache.queue(
				generation,
				model::Id(0),
				crate::cache::Operation::SaveAppPreferences(Box::new(self.current.clone())),
			)
		});
		// A full cache queue must not turn a device preference into a session-only change.
		self.state.dirty = !accepted;
		self.state.saving = accepted;
		self.state.failed = !accepted;
		accepted
	}
	/// Marks changed, valid device preferences for asynchronous persistence.
	pub fn observe(&mut self, ui: &ui::MessagingUi) {
		let value = AppPreferences {
			window_geometry: self.current.window_geometry,
			language: ui.language.preference().map(str::to_owned),
			notifications_enabled: ui.notifications_enabled,
			auto_update: ui.updates.auto_update,
			update_nightly: ui.updates.nightly,
			notification_options: ui.notification_options,
			show_hidden_channels: ui.show_hidden_channels,
			hide_nitro_emojis: ui.hide_nitro_emojis,
			convert_emoticons: ui.convert_emoticons,
			hide_title_bar: ui.hide_title_bar,
			hide_window_decorations: ui.hide_window_decorations,
			gpu_preference: ui.gpu_preference,
			primary_color: ui.primary_color,
			transparency_blur: ui.transparency_blur,
			transparency: ui.transparency,
			blur: ui.blur,
			voice_noise_suppression: ui.voice_processing.effective().suppression
				!= model::voice_settings::NoiseSuppression::Off,
			voice_processing: Some(ui.voice_processing),
			video_settings: ui.video_settings,
			voice_push_to_talk: ui.voice_push_to_talk,
			voice_muted: ui.voice_muted,
			voice_deafened: ui.voice_deafened,
			voice_input: ui.voice_input.clone(),
			voice_output: ui.voice_output.clone(),
			input_percent: ui.voice_gain.input_percent,
			output_percent: ui.voice_gain.output_percent,
			keybinds: ui.keybinds.clone(),
			expanded_folders: ui.expanded_folders.clone(),
			user_volumes: ui.voice_user_volume_overrides(),
			muted_users: ui.voice_user_mutes().to_vec(),
		};
		if value != self.current {
			self.state.touched = true;
			self.state.failed = !value.is_valid();
			if value.is_valid() {
				self.current = value;
				self.state.dirty = true;
			}
		}
	}
	/// Restores saved device preferences, including the opt-in composer conversion.
	pub fn apply(&self, ui: &mut ui::MessagingUi) {
		let value = &self.current;
		ui.language = ui::i18n::Language::from_preference(value.language.as_deref());
		ui::i18n::set_current(ui.language);
		ui.notifications_enabled = value.notifications_enabled;
		ui.updates.auto_update = value.auto_update;
		ui.updates.nightly = value.update_nightly;
		ui.notification_options = value.notification_options;
		ui.show_hidden_channels = value.show_hidden_channels;
		ui.hide_nitro_emojis = value.hide_nitro_emojis;
		ui.convert_emoticons = value.convert_emoticons;
		ui.hide_title_bar = value.hide_title_bar;
		ui.hide_window_decorations = value.hide_window_decorations;
		ui.gpu_preference = value.gpu_preference;
		ui.primary_color = value.primary_color;
		ui.transparency_blur = value.transparency_blur;
		ui.transparency = value.transparency;
		ui.blur = value.blur;
		ui.voice_processing = value.voice_processing.unwrap_or_else(|| {
			model::voice_settings::VoiceProcessing::from_legacy(value.voice_noise_suppression)
		});
		ui.video_settings = value.video_settings;
		ui.voice_push_to_talk = value.voice_push_to_talk;
		ui.voice_muted = value.voice_muted;
		ui.voice_deafened = value.voice_deafened;
		ui.voice_input.clone_from(&value.voice_input);
		ui.voice_output.clone_from(&value.voice_output);
		ui.voice_gain.input_percent = value.input_percent;
		ui.voice_gain.output_percent = value.output_percent;
		ui.keybinds = value.keybinds.clone();
		ui.expanded_folders.clone_from(&value.expanded_folders);
		ui.set_voice_user_volume_overrides(&value.user_volumes);
		ui.set_voice_user_mutes(&value.muted_users);
	}
}

pub fn restore_window_geometry(
	window: &winit::window::Window,
	geometry: local_store::WindowGeometry,
) {
	let saved_monitor = geometry.position.and_then(|position| {
		window.available_monitors().find(|monitor| {
			let origin = monitor.position();
			let size = monitor.size();
			i64::from(position[0]) >= i64::from(origin.x)
				&& i64::from(position[1]) >= i64::from(origin.y)
				&& i64::from(position[0]) < i64::from(origin.x) + i64::from(size.width)
				&& i64::from(position[1]) < i64::from(origin.y) + i64::from(size.height)
		})
	});
	let position = saved_monitor.as_ref().and(geometry.position);
	let Some(monitor) = saved_monitor
		.or_else(|| window.current_monitor())
		.or_else(|| window.available_monitors().next())
	else {
		return;
	};
	let origin = monitor.position();
	let available = monitor.size();
	if available.width == 0 || available.height == 0 {
		return;
	}
	// Move onto the saved monitor first so Windows applies its DPI change before fitting.
	let movable = window.outer_position().is_ok();
	if let (Some(position), true) = (position, movable) {
		window.set_outer_position(winit::dpi::PhysicalPosition::new(position[0], position[1]));
	}
	let scale_factor = if position.is_some() {
		monitor.scale_factor()
	} else {
		window.scale_factor()
	};
	let inner = window.inner_size();
	let outer = window.outer_size();
	let frame = [
		outer.width.saturating_sub(inner.width),
		outer.height.saturating_sub(inner.height),
	];
	// Wayland may not have reported its configured size before the first frame.
	let requested = winit::dpi::LogicalSize::new(geometry.size[0], geometry.size[1])
		.to_physical::<u32>(scale_factor);
	let minimum = window_minimum(available, frame, scale_factor);
	let position = position.or_else(|| {
		window
			.outer_position()
			.ok()
			.map(|position| [position.x, position.y])
	});
	let (position, size) = fit_window_geometry(
		position.unwrap_or([origin.x, origin.y]),
		[
			requested.width.saturating_add(frame[0]),
			requested.height.saturating_add(frame[1]),
		],
		[origin.x, origin.y],
		[available.width, available.height],
		[
			minimum.width.saturating_add(frame[0]),
			minimum.height.saturating_add(frame[1]),
		],
	);
	let size = winit::dpi::PhysicalSize::new(
		size[0].saturating_sub(frame[0]).max(1),
		size[1].saturating_sub(frame[1]).max(1),
	);
	// A newly smaller display must also be allowed to shrink below the usual minimum.
	window.set_min_inner_size(Some(minimum));
	let _ = window.request_inner_size(size);
	if movable {
		window.set_outer_position(winit::dpi::PhysicalPosition::new(position[0], position[1]));
	}
}

pub fn update_window_minimum(
	window: &winit::window::Window,
	monitor: &winit::monitor::MonitorHandle,
) {
	let available = monitor.size();
	if available.width == 0 || available.height == 0 {
		return;
	}
	let inner = window.inner_size();
	let outer = window.outer_size();
	window.set_min_inner_size(Some(window_minimum(
		available,
		[
			outer.width.saturating_sub(inner.width),
			outer.height.saturating_sub(inner.height),
		],
		monitor.scale_factor(),
	)));
}

fn window_minimum(
	available: winit::dpi::PhysicalSize<u32>,
	frame: [u32; 2],
	scale_factor: f64,
) -> winit::dpi::PhysicalSize<u32> {
	let minimum = winit::dpi::LogicalSize::new(MIN_WINDOW_SIZE[0], MIN_WINDOW_SIZE[1])
		.to_physical::<u32>(scale_factor);
	winit::dpi::PhysicalSize::new(
		minimum
			.width
			.min(available.width.saturating_sub(frame[0]).max(1)),
		minimum
			.height
			.min(available.height.saturating_sub(frame[1]).max(1)),
	)
}

/// Fit the complete physical outer rectangle, not just its top-left corner.
fn fit_window_geometry(
	mut position: [i32; 2],
	mut size: [u32; 2],
	origin: [i32; 2],
	available: [u32; 2],
	minimum: [u32; 2],
) -> ([i32; 2], [u32; 2]) {
	for axis in 0..2 {
		size[axis] = size[axis].max(minimum[axis]).min(available[axis].max(1));
		let minimum = i64::from(origin[axis]);
		let maximum = (minimum + i64::from(available[axis].max(1)) - i64::from(size[axis]))
			.min(i64::from(i32::MAX));
		position[axis] = i64::from(position[axis]).clamp(minimum, maximum) as i32;
	}
	(position, size)
}

#[cfg(any(test, all(debug_assertions, feature = "demo")))]
#[cfg_attr(test, test)]
pub fn debug_window_geometry_check() {
	use local_store::{LocalStore, WindowGeometry};
	assert_eq!(
		fit_window_geometry([1850, 1000], [3000, 2000], [0, 0], [1920, 1080], [1, 1]),
		([0, 0], [1920, 1080])
	);
	assert_eq!(
		fit_window_geometry([-100, 900], [1200, 800], [-1920, 0], [1920, 1080], [1, 1]),
		([-1200, 280], [1200, 800])
	);
	assert_eq!(
		fit_window_geometry([-1800, 80], [1000, 700], [-1920, 0], [1920, 1080], [1, 1]),
		([-1800, 80], [1000, 700])
	);
	assert_eq!(
		fit_window_geometry([0, 0], [229, 70], [0, 0], [1920, 1080], MIN_WINDOW_SIZE),
		([0, 0], MIN_WINDOW_SIZE)
	);
	assert_eq!(
		fit_window_geometry([0, 0], [229, 70], [0, 0], [640, 480], MIN_WINDOW_SIZE),
		([0, 0], [640, 480])
	);
	let mut random = [0_u8; 16];
	getrandom::fill(&mut random).unwrap();
	let directory = std::env::temp_dir().join(format!("serein-window-geometry-{random:02x?}"));
	std::fs::create_dir(&directory).unwrap();
	let path = directory.join("preferences.sqlite3");
	let store = LocalStore::open(&path).unwrap();
	assert!(store.app_preferences().unwrap().window_geometry.is_none());
	for position in [None, Some([-1920, 80])] {
		let size = winit::dpi::PhysicalSize::new(1800, 1200).to_logical::<u32>(1.5);
		let geometry = WindowGeometry {
			size: [size.width, size.height],
			position,
		};
		let mut settings = Settings::default();
		settings.current.window_geometry = Some(geometry);
		let mut ui = ui::MessagingUi::default();
		settings.apply(&mut ui);
		ui.notifications_enabled = false;
		settings.observe(&ui);
		assert_eq!(settings.current.window_geometry, Some(geometry));
		store.save_app_preferences(&settings.current).unwrap();
		let reopened = LocalStore::open(&path).unwrap();
		assert_eq!(
			reopened.app_preferences().unwrap().window_geometry,
			Some(geometry)
		);
		for invalid in [
			WindowGeometry {
				size: [0, 800],
				position,
			},
			WindowGeometry {
				size: [16385, 800],
				position,
			},
			WindowGeometry {
				size: [1200, 800],
				position: Some([i32::MAX, 0]),
			},
		] {
			settings.current.window_geometry = Some(invalid);
			assert!(store.save_app_preferences(&settings.current).is_err());
		}
		assert_eq!(
			reopened.app_preferences().unwrap().window_geometry,
			Some(geometry)
		);
	}
	let small = WindowGeometry {
		size: [640, 480],
		position: None,
	};
	let preferences = AppPreferences {
		window_geometry: Some(small),
		..AppPreferences::default()
	};
	store.save_app_preferences(&preferences).unwrap();
	assert_eq!(
		LocalStore::open(&path)
			.unwrap()
			.app_preferences()
			.unwrap()
			.window_geometry,
		Some(small)
	);
	drop(store);
	std::fs::remove_file(path).unwrap();
	std::fs::remove_dir(directory).unwrap();
	println!(
		"Offline window geometry check passed: minimum size, small display, X11 coordinates, native scale, preference preservation, SQLite reopen and bounds."
	);
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn minimum_tracks_monitor_size_scale_and_decorations() {
		for (available, frame, scale, expected) in [
			([640, 480], [16, 39], 1.0, [624, 441]),
			([1920, 1080], [16, 39], 1.0, MIN_WINDOW_SIZE),
			([1920, 1080], [24, 59], 1.5, [1140, 780]),
			([960, 720], [24, 59], 1.5, [936, 661]),
			([8, 12], [16, 39], 1.0, [1, 1]),
		] {
			assert_eq!(
				window_minimum(available.into(), frame, scale),
				winit::dpi::PhysicalSize::from(expected)
			);
		}
	}

	#[test]
	fn startup_defaults_do_not_overwrite_pending_saved_preferences() {
		let mut settings = Settings::default();
		let defaults = AppPreferences::default();
		let mut ui = ui::MessagingUi::default();
		ui.notifications_enabled = defaults.notifications_enabled;
		ui.transparency = defaults.transparency;
		ui.blur = defaults.blur;
		settings.observe(&ui);
		assert!(
			!settings.state.touched,
			"startup defaults must not count as a user edit"
		);
		assert!(!settings.state.dirty);

		settings.current.language = Some("cs".into());
		settings.current.notification_options.current_channel = true;
		settings.loaded = true;
		settings.apply(&mut ui);
		settings.observe(&ui);
		assert_eq!(ui.language, ui::i18n::Language::Czech);
		assert!(ui.notification_options.current_channel);
		assert!(!settings.state.touched);
		assert!(!settings.state.dirty);
	}

	#[test]
	fn video_settings_are_observed_and_restored() {
		use model::voice_settings::{
			VideoBackend, VideoCodec, VideoFrameRate, VideoResolution, VideoSettings,
		};
		let mut settings = Settings::default();
		let mut ui = ui::MessagingUi::default();
		ui.video_settings = VideoSettings {
			backend: VideoBackend::Experimental,
			codec: VideoCodec::H265,
			camera_resolution: VideoResolution::P4320,
			camera_frame_rate: VideoFrameRate::Fps60,
		};
		settings.observe(&ui);
		assert_eq!(settings.current.video_settings, ui.video_settings);
		assert!(settings.state.dirty);
		ui.video_settings = VideoSettings::default();
		settings.apply(&mut ui);
		assert_eq!(ui.video_settings, settings.current.video_settings);
	}

	#[test]
	fn legacy_preferences_without_voice_settings_keep_suppression_disabled() {
		let current: AppPreferences = serde_json::from_str("{}").unwrap();
		assert!(!current.voice_noise_suppression);
		assert!(current.voice_processing.is_none());
		assert_eq!(
			current.video_settings,
			model::voice_settings::VideoSettings::default()
		);
		let settings = Settings {
			current,
			..Default::default()
		};
		let mut ui = ui::MessagingUi::default();
		settings.apply(&mut ui);
		assert_eq!(
			ui.voice_processing.effective().suppression,
			model::voice_settings::NoiseSuppression::Off
		);
	}
}
