use local_store::AppPreferences;

pub const MIN_WINDOW_SIZE: [u32; 2] = [760, 520];
pub const DEFAULT_WINDOW_SIZE: [u32; 2] = [1120, 760];
pub const MIN_POPOUT_SIZE: [u32; 2] = [480, 320];
pub const DEFAULT_POPOUT_SIZE: [u32; 2] = [960, 640];
/// Decoration allowance (logical) reserved when fitting the call window before it exists:
/// covers Windows' 8 px borders and title bar and typical X11/macOS title bars.
pub const POPOUT_FRAME: [u32; 2] = [16, 40];

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
			preload_on_hover: ui.preload_on_hover,
			auto_idle: ui.auto_idle,
			hide_title_bar: ui.hide_title_bar,
			message_requests_collapsed: ui.message_requests_collapsed,
			hide_window_decorations: ui.hide_window_decorations,
			custom_title_bar: ui.custom_title_bar,
			gpu_preference: ui.gpu_preference,
			steady_refresh_rate: ui.steady_refresh_rate,
			primary_color: ui.primary_color,
			transparency_blur: ui.transparency_blur,
			transparency: ui.transparency,
			blur: ui.blur,
			voice_noise_suppression: ui.voice_processing.effective().suppression
				!= model::voice_settings::NoiseSuppression::Off,
			voice_processing: Some(ui.voice_processing),
			voice_push_to_talk: ui.voice_push_to_talk,
			voice_muted: ui.voice_muted,
			voice_deafened: ui.voice_deafened,
			voice_popout_auto: ui.voice_popout_auto,
			voice_popout_geometry: self.current.voice_popout_geometry,
			voice_input: ui.voice_input.clone(),
			voice_output: ui.voice_output.clone(),
			input_percent: ui.voice_gain.input_percent,
			output_percent: ui.voice_gain.output_percent,
			keybinds: ui.keybinds.clone(),
			expanded_folders: ui.expanded_folders.clone(),
			user_volumes: ui.voice_user_volume_overrides(),
			muted_users: ui.voice_user_mutes().to_vec(),
			cosmetics: ui.cosmetics,
			image_cache: ui.image_cache,
			time_format: ui.time_format,
			plaintext_login: ui.plaintext_login,
			diagnostics_log: ui.diagnostics_log,
			setup_done: !ui.setup_pending,
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
		ui.preload_on_hover = value.preload_on_hover;
		ui.auto_idle = value.auto_idle;
		ui.hide_title_bar = value.hide_title_bar;
		ui.message_requests_collapsed = value.message_requests_collapsed;
		ui.hide_window_decorations = value.hide_window_decorations;
		ui.custom_title_bar = value.custom_title_bar;
		ui.gpu_preference = value.gpu_preference;
		ui.steady_refresh_rate = value.steady_refresh_rate;
		ui.primary_color = value.primary_color;
		ui.transparency_blur = value.transparency_blur;
		ui.transparency = value.transparency;
		ui.blur = value.blur;
		ui.voice_processing = value.voice_processing.unwrap_or_else(|| {
			model::voice_settings::VoiceProcessing::from_legacy(value.voice_noise_suppression)
		});
		ui.voice_push_to_talk = value.voice_push_to_talk;
		ui.voice_muted = value.voice_muted;
		ui.voice_deafened = value.voice_deafened;
		ui.voice_popout_auto = value.voice_popout_auto;
		ui.voice_input.clone_from(&value.voice_input);
		ui.voice_output.clone_from(&value.voice_output);
		ui.voice_gain.input_percent = value.input_percent;
		ui.voice_gain.output_percent = value.output_percent;
		ui.keybinds = value.keybinds.clone();
		ui.expanded_folders.clone_from(&value.expanded_folders);
		ui.set_voice_user_volume_overrides(&value.user_volumes);
		ui.set_voice_user_mutes(&value.muted_users);
		ui.cosmetics = value.cosmetics;
		ui.image_cache = value.image_cache;
		ui.time_format = value.time_format;
		ui.plaintext_login = value.plaintext_login;
		ui.diagnostics_log = value.diagnostics_log;
		// Unreadable preferences fall back to defaults; never nag with setup on every launch.
		ui.setup_pending = self.loaded && !value.setup_done;
		ui::local_time::set_format(ui.time_format, ui.language);
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
	// Linux maximizes from the viewport builder. macOS and Windows maximize only after placing the
	// normal geometry, since moving a maximized window resets it.
	if geometry.maximized {
		#[cfg(target_os = "windows")]
		egui_winit::set_maximized(window, true); // Keeps the still-hidden window from flashing.
		#[cfg(target_os = "macos")]
		window.set_maximized(true);
	}
}

type Monitor = (
	winit::dpi::PhysicalPosition<i32>,
	winit::dpi::PhysicalSize<u32>,
	f64,
);

/// Whether a top-left corner lies inside this monitor's bounds.
fn monitor_contains(monitor: Monitor, position: [i32; 2]) -> bool {
	let (origin, size, _) = monitor;
	i64::from(position[0]) >= i64::from(origin.x)
		&& i64::from(position[1]) >= i64::from(origin.y)
		&& i64::from(position[0]) < i64::from(origin.x) + i64::from(size.width)
		&& i64::from(position[1]) < i64::from(origin.y) + i64::from(size.height)
}

/// The current display as origin, size and scale; a window off every display has none.
fn monitor_of(window: &winit::window::Window) -> Option<Monitor> {
	window
		.current_monitor()
		.map(|monitor| (monitor.position(), monitor.size(), monitor.scale_factor()))
}

/// First placement of the call window: the saved spot while its display is still connected,
/// otherwise the saved (or a first-use) size centred on the main window's monitor.
pub fn fit_popout_geometry(
	window: &winit::window::Window,
	saved: Option<local_store::WindowGeometry>,
) -> local_store::WindowGeometry {
	let monitors: Vec<_> = window
		.available_monitors()
		.map(|monitor| (monitor.position(), monitor.size(), monitor.scale_factor()))
		.collect();
	let mut geometry = fit_popout_geometry_on(&monitors, monitor_of(window), saved);
	if window.outer_position().is_err() {
		// Wayland and other platforms place the window themselves and report no position.
		geometry.position = None;
	}
	geometry
}

/// The display a fitted popout position sits on; the viewport builder takes points, not pixels.
pub fn popout_scale_factor(window: &winit::window::Window, position: [i32; 2]) -> f64 {
	window
		.available_monitors()
		.find(|monitor| {
			monitor_contains(
				(monitor.position(), monitor.size(), monitor.scale_factor()),
				position,
			)
		})
		.map_or(1.0, |monitor| monitor.scale_factor())
}

/// Fits the saved or first-use size inside the saved position's display, a monitorless fallback
/// included; the returned position is centred when nothing was saved.
fn fit_popout_geometry_on(
	monitors: &[Monitor],
	current: Option<Monitor>,
	saved: Option<local_store::WindowGeometry>,
) -> local_store::WindowGeometry {
	let default = local_store::WindowGeometry {
		size: DEFAULT_POPOUT_SIZE,
		position: None,
		maximized: false,
	};
	let saved_position = saved.and_then(|geometry| geometry.position);
	let Some(monitor) = saved_position
		.and_then(|position| {
			monitors
				.iter()
				.copied()
				.find(|monitor| monitor_contains(*monitor, position))
		})
		.or(current)
		.or_else(|| monitors.first().copied())
	else {
		return saved.unwrap_or(default);
	};
	let (origin, available, scale) = monitor;
	if available.width == 0 || available.height == 0 {
		return saved.unwrap_or(default);
	}
	let [mut width, mut height] = saved.map_or(DEFAULT_POPOUT_SIZE, |geometry| geometry.size);
	// The window does not exist yet, so its decorations are unknown; reserve a typical frame
	// so the outer rectangle, not just the content, fits the display.
	let frame =
		winit::dpi::LogicalSize::new(POPOUT_FRAME[0], POPOUT_FRAME[1]).to_physical::<u32>(scale);
	let limit = |available: u32, frame: u32| {
		(f64::from(available.saturating_sub(frame)) / scale)
			.floor()
			.max(1.0) as u32
	};
	// Never below the window minimum, never past the display edge.
	width = width.clamp(
		MIN_POPOUT_SIZE[0].min(limit(available.width, frame.width)),
		limit(available.width, frame.width),
	);
	height = height.clamp(
		MIN_POPOUT_SIZE[1].min(limit(available.height, frame.height)),
		limit(available.height, frame.height),
	);
	let inner = winit::dpi::LogicalSize::new(width, height).to_physical::<u32>(scale);
	let physical = [
		inner.width.saturating_add(frame.width),
		inner.height.saturating_add(frame.height),
	];
	let centred = [
		origin.x + ((i64::from(available.width) - i64::from(physical[0])) / 2) as i32,
		origin.y + ((i64::from(available.height) - i64::from(physical[1])) / 2) as i32,
	];
	let (position, _) = fit_window_geometry(
		saved_position
			.filter(|position| monitor_contains(monitor, *position))
			.unwrap_or(centred),
		physical,
		[origin.x, origin.y],
		[available.width, available.height],
		physical,
	);
	local_store::WindowGeometry {
		size: [width, height],
		position: (saved.is_none() || saved_position.is_some()).then_some(position),
		maximized: saved.is_some_and(|geometry| geometry.maximized),
	}
}

/// Viewport rectangles are in points (physical pixels ÷ native scale ÷ egui zoom) while the
/// saved geometry keeps a logical size and a physical position. Sizes come from the content
/// rectangle because Wayland reports no inner rectangle.
pub fn popout_frame_geometry(
	size_points: Option<[f32; 2]>,
	position_points: Option<[f32; 2]>,
	native_scale: f64,
	zoom: f32,
) -> (Option<[u32; 2]>, Option<[i32; 2]>) {
	let zoom = f64::from(zoom.max(0.01));
	(
		size_points.map(|size| {
			[
				(size[0] as f64 * zoom).round().max(1.0) as u32,
				(size[1] as f64 * zoom).round().max(1.0) as u32,
			]
		}),
		position_points.map(|position| {
			[
				(position[0] as f64 * native_scale * zoom).round() as i32,
				(position[1] as f64 * native_scale * zoom).round() as i32,
			]
		}),
	)
}

/// Builder points from saved geometry: the viewport builder scales sizes and positions by
/// egui zoom on the way out, so a saved logical size and physical position come back here.
pub fn popout_request_geometry(
	size: [u32; 2],
	position: Option<[i32; 2]>,
	native_scale: f64,
	zoom: f32,
) -> ([f32; 2], Option<[f32; 2]>) {
	let zoom = f64::from(zoom.max(0.01));
	let scale = native_scale.max(0.01) * zoom;
	(
		[size[0] as f32 / zoom as f32, size[1] as f32 / zoom as f32],
		position.map(|position| {
			[
				(position[0] as f64 / scale) as f32,
				(position[1] as f64 / scale) as f32,
			]
		}),
	)
}

/// The geometry a call-window frame should persist, if it reports enough to save one.
/// A maximized window keeps its last normal size and position and only flips the flag;
/// fullscreen and minimized frames never overwrite the saved geometry.
pub fn sample_popout_geometry(
	last: Option<local_store::WindowGeometry>,
	size: Option<[u32; 2]>,
	position: Option<[i32; 2]>,
	maximized: Option<bool>,
	fullscreen: Option<bool>,
	minimized: Option<bool>,
) -> Option<local_store::WindowGeometry> {
	if fullscreen == Some(true) || minimized == Some(true) {
		return None;
	}
	if maximized == Some(true) {
		return last.map(|geometry| local_store::WindowGeometry {
			maximized: true,
			..geometry
		});
	}
	Some(local_store::WindowGeometry {
		size: size?,
		position,
		maximized: false,
	})
}

/// X11 window managers ignore maximize requests for a window that is not yet shown.
pub fn maximize_after_show(window: &winit::window::Window) -> bool {
	#[cfg(target_os = "linux")]
	{
		use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
		matches!(
			window.window_handle().map(|h| h.as_raw()),
			Ok(RawWindowHandle::Xlib(_) | RawWindowHandle::Xcb(_))
		)
	}
	#[cfg(not(target_os = "linux"))]
	{
		let _ = window;
		false
	}
}

pub fn update_window_minimum(
	window: &winit::window::Window,
	monitor: &winit::monitor::MonitorHandle,
	applied: &mut Option<winit::dpi::PhysicalSize<u32>>,
) {
	let available = monitor.size();
	if available.width == 0 || available.height == 0 {
		return;
	}
	let inner = window.inner_size();
	let outer = window.outer_size();
	let minimum = window_minimum(
		available,
		[
			outer.width.saturating_sub(inner.width),
			outer.height.saturating_sub(inner.height),
		],
		monitor.scale_factor(),
	);
	// Windows applies a minimum synchronously and invalidates the whole window, so a redundant
	// call forces a frame; this runs whenever the window rectangle changes, including a drag.
	if *applied == Some(minimum) {
		return;
	}
	window.set_min_inner_size(Some(minimum));
	*applied = Some(minimum);
}

/// A placement must stay put this long before it is saved; a drag samples a new one per frame.
pub const GEOMETRY_REST_DELAY: std::time::Duration = std::time::Duration::from_millis(500);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Observation {
	/// The saved placement already matches the sample.
	Saved,
	/// The placement has not yet stayed unchanged for the rest interval.
	Pending,
	/// The placement stayed unchanged for the rest interval and can be saved.
	Commit,
}

/// Saves a window placement only after a full interval without a sampled change.
#[derive(Default)]
pub struct GeometryRest {
	pending: Option<(local_store::WindowGeometry, std::time::Instant)>,
	/// When a frame should sample the pending placement again, as of this frame's observation.
	/// [`Self::begin_frame`] clears it, so a frame that samples nothing cannot re-arm it.
	wake: Option<std::time::Duration>,
}

impl GeometryRest {
	/// Starts a frame's sampling: only an observation in this frame schedules the next wake.
	pub fn begin_frame(&mut self) {
		self.wake = None;
	}
	/// Observe placement at a monotonic time; early frames do not shorten the rest interval.
	pub fn observe(
		&mut self,
		sampled: local_store::WindowGeometry,
		saved: Option<local_store::WindowGeometry>,
		now: std::time::Instant,
	) -> Observation {
		self.wake = None;
		if saved == Some(sampled) {
			self.pending = None;
			return Observation::Saved;
		}
		if let Some((pending, changed_at)) = self.pending
			&& pending == sampled
		{
			let rested = now.saturating_duration_since(changed_at);
			if rested < GEOMETRY_REST_DELAY {
				self.wake = Some(GEOMETRY_REST_DELAY - rested);
				return Observation::Pending;
			}
			self.pending = None;
			return Observation::Commit;
		}
		self.pending = Some((sampled, now));
		self.wake = Some(GEOMETRY_REST_DELAY);
		Observation::Pending
	}
	/// How long until a frame should sample the pending placement again. Every pass of the
	/// observing frame may request it, since a discarded pass's request is ignored.
	pub fn wake(&self) -> Option<std::time::Duration> {
		self.wake
	}
	/// Keep the latest normal placement when a maximize event interrupts its rest interval.
	pub fn maximized_geometry(
		&self,
		saved: Option<local_store::WindowGeometry>,
	) -> local_store::WindowGeometry {
		let normal = self.pending.map(|(geometry, _)| geometry).or(saved);
		local_store::WindowGeometry {
			maximized: true,
			..normal.unwrap_or(local_store::WindowGeometry {
				size: DEFAULT_WINDOW_SIZE,
				position: None,
				maximized: true,
			})
		}
	}

	/// The last sample, for a window that closes or hides before it could rest.
	pub fn take_pending(&mut self) -> Option<local_store::WindowGeometry> {
		self.wake = None;
		self.pending.take().map(|(geometry, _)| geometry)
	}
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
	let monitors = [
		(
			winit::dpi::PhysicalPosition::new(0, 0),
			winit::dpi::PhysicalSize::new(1920, 1080),
			1.0,
		),
		(
			winit::dpi::PhysicalPosition::new(1920, 0),
			winit::dpi::PhysicalSize::new(2560, 1440),
			1.0,
		),
	];
	let current = Some(monitors[0]);
	// The outer rectangle (content plus the reserved frame) stays on its display.
	let outer_fits = |geometry: WindowGeometry| {
		let [x, y] = geometry.position.unwrap();
		let outer = [
			geometry.size[0] + POPOUT_FRAME[0],
			geometry.size[1] + POPOUT_FRAME[1],
		];
		monitors.iter().any(|(origin, size, _)| {
			x >= origin.x
				&& y >= origin.y
				&& i64::from(x) + i64::from(outer[0]) <= i64::from(origin.x) + i64::from(size.width)
				&& i64::from(y) + i64::from(outer[1])
					<= i64::from(origin.y) + i64::from(size.height)
		})
	};
	assert_eq!(
		fit_popout_geometry_on(&monitors, current, None),
		WindowGeometry {
			size: DEFAULT_POPOUT_SIZE,
			position: Some([472, 200]),
			maximized: false,
		}
	);
	// A display that is no longer connected still opens on a visible one.
	let lost = WindowGeometry {
		size: [1200, 800],
		position: Some([5000, 3000]),
		maximized: true,
	};
	let fitted = fit_popout_geometry_on(&monitors, current, Some(lost));
	assert_eq!(fitted.size, [1200, 800]);
	assert_eq!(fitted.position, Some([352, 120]));
	assert!(outer_fits(fitted));
	assert!(fitted.maximized);
	assert_eq!(
		fit_popout_geometry_on(
			&monitors,
			current,
			Some(WindowGeometry {
				size: [800, 600],
				position: Some([2200, 100]),
				maximized: false,
			})
		),
		WindowGeometry {
			size: [800, 600],
			position: Some([2200, 100]),
			maximized: false,
		}
	);
	// Wayland reports no position, so size-only geometry keeps it absent.
	assert_eq!(
		fit_popout_geometry_on(
			&monitors,
			current,
			Some(WindowGeometry {
				size: [800, 600],
				position: None,
				maximized: false,
			})
		),
		WindowGeometry {
			size: [800, 600],
			position: None,
			maximized: false,
		}
	);
	// A saved size larger than the display lands fully inside it, frame included.
	let oversized = fit_popout_geometry_on(
		&monitors,
		current,
		Some(WindowGeometry {
			size: [4000, 3000],
			position: Some([1800, 1000]),
			maximized: false,
		}),
	);
	assert_eq!(
		oversized,
		WindowGeometry {
			size: [1904, 1040],
			position: Some([0, 0]),
			maximized: false,
		}
	);
	assert!(outer_fits(oversized));
	// A maximized frame keeps the last normal size and position and only flips the flag.
	let normal = WindowGeometry {
		size: [800, 600],
		position: Some([1200, 200]),
		maximized: false,
	};
	assert_eq!(
		sample_popout_geometry(
			Some(normal),
			Some([2560, 1440]),
			Some([0, 0]),
			Some(true),
			Some(false),
			Some(false),
		),
		Some(WindowGeometry {
			maximized: true,
			..normal
		})
	);
	// Fullscreen and minimized frames never overwrite the saved geometry.
	for (fullscreen, minimized) in [(Some(true), Some(false)), (Some(false), Some(true))] {
		assert_eq!(
			sample_popout_geometry(
				Some(normal),
				Some([2560, 1440]),
				Some([0, 0]),
				Some(false),
				fullscreen,
				minimized,
			),
			None
		);
	}
	// A normal frame saves its own logical size and physical position.
	assert_eq!(
		sample_popout_geometry(
			Some(WindowGeometry {
				maximized: true,
				..normal
			}),
			Some([900, 700]),
			Some([300, 100]),
			Some(false),
			Some(false),
			Some(false),
		),
		Some(WindowGeometry {
			size: [900, 700],
			position: Some([300, 100]),
			maximized: false,
		})
	);
	// egui zoom divides every viewport report; stored geometry keeps logical and physical.
	let (size, position) =
		popout_frame_geometry(Some([640.0, 480.0]), Some([200.0, 100.0]), 1.0, 1.5);
	assert_eq!(size, Some([960, 720]));
	assert_eq!(position, Some([300, 150]));
	assert_eq!(
		popout_request_geometry([960, 720], Some([300, 150]), 1.0, 1.5),
		([640.0, 480.0], Some([200.0, 100.0]))
	);
	assert_eq!(
		popout_request_geometry([960, 720], Some([300, 150]), 2.0, 1.5),
		([640.0, 480.0], Some([100.0, 50.0]))
	);
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
			maximized: position.is_some(),
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
				..geometry
			},
			WindowGeometry {
				size: [16385, 800],
				..geometry
			},
			WindowGeometry {
				position: Some([i32::MAX, 0]),
				..geometry
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
	// Older records were saved only while not maximized.
	let small: WindowGeometry =
		serde_json::from_str(r#"{"size":[640,480],"position":null}"#).unwrap();
	assert!(!small.maximized);
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
		"Offline window geometry check passed: minimum size, small display, X11 coordinates, native scale, maximized state, older records, preference preservation, SQLite reopen and bounds."
	);
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn startup_defaults_do_not_overwrite_pending_saved_preferences() {
		let mut settings = Settings::default();
		let defaults = AppPreferences::default();
		let mut ui = ui::MessagingUi::default();
		ui.notifications_enabled = defaults.notifications_enabled;
		ui.transparency = defaults.transparency;
		ui.blur = defaults.blur;
		ui.preload_on_hover = defaults.preload_on_hover;
		ui.auto_idle = defaults.auto_idle;
		ui.custom_title_bar = defaults.custom_title_bar;
		ui.setup_pending = !defaults.setup_done;
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
	fn collapsed_message_requests_restore_and_track_user_changes() {
		let mut settings = Settings {
			loaded: true,
			..Default::default()
		};
		settings.current.message_requests_collapsed = true;
		let mut ui = ui::MessagingUi::default();
		settings.apply(&mut ui);
		assert!(ui.message_requests_collapsed);
		settings.observe(&ui);
		assert!(!settings.state.dirty);

		ui.message_requests_collapsed = false;
		settings.observe(&ui);
		assert!(!settings.current.message_requests_collapsed);
		assert!(settings.state.dirty);
	}

	#[test]
	fn popout_preferences_round_trip_and_keep_saved_geometry() {
		let mut settings = Settings {
			loaded: true,
			..Default::default()
		};
		let geometry = local_store::WindowGeometry {
			size: [960, 640],
			position: Some([-1920, 0]),
			maximized: true,
		};
		settings.current.voice_popout_geometry = Some(geometry);
		let mut ui = ui::MessagingUi::default();
		settings.apply(&mut ui);
		assert!(!ui.voice_popout_auto);
		ui.voice_popout_auto = true;
		settings.observe(&ui);
		assert!(settings.current.voice_popout_auto);
		assert_eq!(settings.current.voice_popout_geometry, Some(geometry));

		settings.current.voice_popout_auto = false;
		settings.apply(&mut ui);
		assert!(!ui.voice_popout_auto);
	}

	#[test]
	fn legacy_preferences_without_voice_settings_keep_suppression_disabled() {
		let current: AppPreferences = serde_json::from_str("{}").unwrap();
		assert!(!current.voice_noise_suppression);
		assert!(current.voice_processing.is_none());
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

	#[test]
	fn window_geometry_is_saved_only_after_it_rests() {
		let mut rest = GeometryRest::default();
		let now = std::time::Instant::now();
		let placed = local_store::WindowGeometry {
			size: [1120, 760],
			position: Some([100, 100]),
			maximized: false,
		};
		let mut moving = placed;
		for step in 0..8 {
			moving.position = Some([100 + step * 7, 100]);
			assert_eq!(rest.observe(moving, None, now), Observation::Pending);
		}
		assert_eq!(rest.observe(placed, Some(placed), now), Observation::Saved);
		assert_eq!(rest.take_pending(), None);

		assert_eq!(rest.observe(placed, None, now), Observation::Pending);
		assert_eq!(
			rest.observe(placed, None, now + GEOMETRY_REST_DELAY / 2),
			Observation::Pending
		);
		// Every pass of the observing frame may request the wake, so a discarded pass keeps it;
		// the next frame that samples nothing drops it.
		assert_eq!(rest.wake(), Some(GEOMETRY_REST_DELAY / 2));
		assert_eq!(rest.wake(), Some(GEOMETRY_REST_DELAY / 2));
		rest.begin_frame();
		assert_eq!(rest.wake(), None);
		assert_eq!(
			rest.observe(moving, None, now + GEOMETRY_REST_DELAY / 2),
			Observation::Pending
		);
		assert_eq!(
			rest.observe(moving, None, now + GEOMETRY_REST_DELAY),
			Observation::Pending
		);
		// A change restarts the interval, so the wake moves with it.
		assert_eq!(rest.wake(), Some(GEOMETRY_REST_DELAY / 2));
		assert_eq!(
			rest.observe(moving, None, now + GEOMETRY_REST_DELAY * 2),
			Observation::Commit
		);
		assert_eq!(rest.wake(), None);
		assert_eq!(rest.observe(placed, Some(placed), now), Observation::Saved);
		assert_eq!(rest.take_pending(), None);

		assert_eq!(rest.observe(moving, None, now), Observation::Pending);
		let maximized = rest.maximized_geometry(Some(placed));
		assert_eq!(
			maximized,
			local_store::WindowGeometry {
				maximized: true,
				..moving
			}
		);
		assert_eq!(
			rest.observe(maximized, Some(placed), now),
			Observation::Pending
		);
		assert_eq!(rest.maximized_geometry(Some(placed)), maximized);
		assert_eq!(
			rest.observe(maximized, Some(placed), now + GEOMETRY_REST_DELAY),
			Observation::Commit
		);
		assert_eq!(rest.take_pending(), None);
		assert_eq!(rest.observe(moving, None, now), Observation::Pending);
		assert_eq!(rest.take_pending(), Some(moving));
		assert_eq!(rest.take_pending(), None);
	}
}
