//! ext-background-effect-v1 on winit's existing Wayland connection.
#![allow(unsafe_code)]

use wayland_client::{
	Connection, Dispatch, EventQueue, Proxy, QueueHandle, WEnum, delegate_noop,
	protocol::{wl_compositor, wl_region, wl_registry, wl_surface},
};
use wayland_protocols::ext::background_effect::v1::client::{
	ext_background_effect_manager_v1::{self as manager, ExtBackgroundEffectManagerV1},
	ext_background_effect_surface_v1::ExtBackgroundEffectSurfaceV1,
};
use winit::raw_window_handle::{
	HasDisplayHandle, HasWindowHandle, RawDisplayHandle, RawWindowHandle,
};

pub(super) struct Blur {
	effect: Option<ExtBackgroundEffectSurfaceV1>,
	state: State,
	queue: EventQueue<State>,
	connection: Connection,
	applied: bool,
	/// Surface-local `[width, height, radius]` of a rounded window; `None` blurs every pixel.
	shape: Option<[i32; 3]>,
	applied_shape: Option<[i32; 3]>,
}

#[derive(Default)]
struct State {
	manager: Option<ExtBackgroundEffectManagerV1>,
	compositor: Option<wl_compositor::WlCompositor>,
	blur_supported: bool,
}

impl Blur {
	pub(super) fn new(window: &winit::window::Window) -> Option<Self> {
		let RawDisplayHandle::Wayland(display) = window.display_handle().ok()?.as_raw() else {
			return None;
		};
		let RawWindowHandle::Wayland(handle) = window.window_handle().ok()?.as_raw() else {
			return None;
		};
		// SAFETY: the parent Blur retains the winit window until this guest connection and
		// its proxies are dropped. We neither disconnect the display nor own its surface.
		let backend = unsafe {
			wayland_backend::sys::client::Backend::from_foreign_display(
				display.display.as_ptr().cast(),
			)
		};
		let connection = Connection::from_backend(backend);
		let queue = connection.new_event_queue();
		let mut blur = Self {
			effect: None,
			state: State::default(),
			queue,
			connection,
			applied: false,
			shape: None,
			applied_shape: None,
		};
		// Only two fixed globals are retained; initialization roundtrips never run in rendering.
		let _registry = blur
			.connection
			.display()
			.get_registry(&blur.queue.handle(), ());
		blur.queue.roundtrip(&mut blur.state).ok()?;
		blur.state.manager.as_ref()?;
		blur.state.compositor.as_ref()?;
		blur.queue.roundtrip(&mut blur.state).ok()?;
		// SAFETY: same live borrowed wl_surface, used only to attach our extension.
		let id = unsafe {
			wayland_backend::sys::client::ObjectId::from_ptr(
				wl_surface::WlSurface::interface(),
				handle.surface.as_ptr().cast(),
			)
		}
		.ok()?;
		let surface = wl_surface::WlSurface::from_id(&blur.connection, id).ok()?;
		blur.effect = Some(blur.state.manager.as_ref()?.get_background_effect(
			&surface,
			&blur.queue.handle(),
			(),
		));
		Some(blur)
	}

	pub(super) fn set_shape(&mut self, shape: Option<[i32; 3]>) {
		self.shape = shape;
	}

	/// Dispatch only already-read events; winit owns socket reads and surface commits.
	pub(super) fn set_enabled(&mut self, enabled: bool) -> bool {
		if self.queue.dispatch_pending(&mut self.state).is_err() {
			return false;
		}
		let supported = self.state.blur_supported;
		let enabled = enabled && supported;
		if enabled != self.applied || (enabled && self.shape != self.applied_shape) {
			let effect = self.effect.as_ref().expect("initialized background effect");
			if enabled {
				let region = self
					.state
					.compositor
					.as_ref()
					.expect("bound compositor")
					.create_region(&self.queue.handle(), ());
				if let Some([width, height, radius]) = self.shape {
					// Rounded window: one scanline per corner row, so the blur stops at the edge.
					let radius = radius.clamp(0, width.min(height) / 2);
					for row in 0..radius {
						let dy = f64::from(radius - row) - 0.5;
						let dx = (f64::from(radius).powi(2) - dy * dy).max(0.0).sqrt();
						let inset = radius - dx.round() as i32;
						region.add(inset, row, width - 2 * inset, 1);
						region.add(inset, height - 1 - row, width - 2 * inset, 1);
					}
					region.add(0, radius, width, height - 2 * radius);
				} else {
					// Surface-local region is clipped by the compositor, including after resizing.
					region.add(0, 0, i32::MAX, i32::MAX);
				}
				effect.set_blur_region(Some(&region));
				region.destroy();
			} else {
				effect.set_blur_region(None);
			}
			self.applied = enabled;
			self.applied_shape = self.shape;
			// Do not commit a winit-owned surface here; its next rendered frame applies this.
			let _ = self.connection.flush();
		}
		supported
	}
}

impl Drop for Blur {
	fn drop(&mut self) {
		if let Some(effect) = &self.effect {
			effect.destroy();
		}
		if let Some(manager) = &self.state.manager {
			manager.destroy();
		}
		let _ = self.connection.flush();
	}
}

impl Dispatch<wl_registry::WlRegistry, ()> for State {
	fn event(
		state: &mut Self,
		registry: &wl_registry::WlRegistry,
		event: wl_registry::Event,
		_: &(),
		_: &Connection,
		qh: &QueueHandle<Self>,
	) {
		if let wl_registry::Event::Global {
			name,
			interface,
			version,
		} = event
		{
			if version >= 1
				&& interface == "ext_background_effect_manager_v1"
				&& state.manager.is_none()
			{
				state.manager = Some(registry.bind(name, 1, qh, ()));
			} else if version >= 1 && interface == "wl_compositor" && state.compositor.is_none() {
				state.compositor = Some(registry.bind(name, 1, qh, ()));
			}
		}
	}
}
impl Dispatch<ExtBackgroundEffectManagerV1, ()> for State {
	fn event(
		state: &mut Self,
		_: &ExtBackgroundEffectManagerV1,
		event: manager::Event,
		_: &(),
		_: &Connection,
		_: &QueueHandle<Self>,
	) {
		if let manager::Event::Capabilities { flags } = event {
			state.blur_supported = match flags {
				WEnum::Value(flags) => flags.contains(manager::Capability::Blur),
				WEnum::Unknown(flags) => flags & manager::Capability::Blur.bits() != 0,
			};
		}
	}
}
delegate_noop!(State: ignore wl_compositor::WlCompositor);
delegate_noop!(State: ignore wl_region::WlRegion);
delegate_noop!(State: ignore ExtBackgroundEffectSurfaceV1);
