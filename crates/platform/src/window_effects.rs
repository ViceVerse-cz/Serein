//! Native background blur; transparency itself remains owned by winit/eframe.
use std::sync::Arc;
use winit::window::Window;

#[cfg(target_os = "linux")]
mod wayland;
#[cfg(target_os = "linux")]
mod x11;

pub struct Blur {
	#[cfg(target_os = "linux")]
	wayland: Option<wayland::Blur>,
	native_enabled: bool,
	// Keep the native display and surface alive until the protocol objects are dropped.
	window: Arc<Window>,
}

impl Blur {
	/// Initialize once on the window thread, before rendering starts.
	pub fn new(window: Arc<Window>) -> Self {
		Self {
			#[cfg(target_os = "linux")]
			wayland: wayland::Blur::new(&window),
			native_enabled: false,
			window,
		}
	}

	pub fn set_enabled(&mut self, enabled: bool) {
		#[cfg(target_os = "linux")]
		let enabled = if let Some(wayland) = &mut self.wayland {
			// Prefer the standard protocol; retain winit's KDE fallback without stacking effects.
			let supported = wayland.set_enabled(enabled);
			enabled && !supported
		} else {
			enabled
		};
		if enabled == self.native_enabled {
			return;
		}
		self.native_enabled = enabled;
		#[cfg(target_os = "windows")]
		{
			use winit::platform::windows::{BackdropType, WindowExtWindows};
			self.window.set_system_backdrop(if enabled {
				BackdropType::TransientWindow
			} else {
				BackdropType::None
			});
		}
		#[cfg(target_os = "linux")]
		{
			use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
			if matches!(
				self.window.window_handle().map(|h| h.as_raw()),
				Ok(RawWindowHandle::Xlib(_))
			) {
				if let Err(error) = x11::set_blur(&self.window, enabled) {
					eprintln!("Window blur: {error}");
				}
			} else {
				self.window.set_blur(enabled);
			}
		}
		#[cfg(not(any(target_os = "linux", target_os = "windows")))]
		self.window.set_blur(enabled);
	}
}
