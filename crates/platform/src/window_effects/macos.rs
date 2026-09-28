//! Behind-window blur as a native effect view under the rendered content.
//!
//! winit's blur asks the window server to blur the whole window backing, which ignores the
//! window's rounded corner mask: on recent macOS the blur spills past the rounded frame. An
//! effect view in the frame view sits inside that mask, so blur and content share one shape.
//! Its radius is fixed by the system, like every compositor's, so Appearance offers a switch.
#![allow(unsafe_code)]

use objc2::rc::Retained;
use objc2::{MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{
	NSAutoresizingMaskOptions, NSView, NSVisualEffectBlendingMode, NSVisualEffectMaterial,
	NSVisualEffectState, NSVisualEffectView, NSWindowOrderingMode,
};
use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
use winit::window::Window;

pub struct Blur {
	view: Retained<NSVisualEffectView>,
}

impl Blur {
	pub fn new(window: &Window) -> Option<Self> {
		let main = MainThreadMarker::new()?;
		let RawWindowHandle::AppKit(handle) = window.window_handle().ok()?.as_raw() else {
			return None;
		};
		// SAFETY: winit owns this NSView for the borrowed window's lifetime and AppKit access
		// stays on the main thread, checked above.
		let content = unsafe { handle.ns_view.cast::<NSView>().as_ref() };
		// SAFETY: the content view is installed in its window, whose frame view outlives it.
		let frame = unsafe { content.superview() }?;
		let view =
			NSVisualEffectView::initWithFrame(NSVisualEffectView::alloc(main), frame.bounds());
		view.setBlendingMode(NSVisualEffectBlendingMode::BehindWindow);
		view.setMaterial(NSVisualEffectMaterial::UnderWindowBackground);
		// Stay blurred while the window is inactive, as the window-server blur did.
		view.setState(NSVisualEffectState::Active);
		view.setAutoresizingMask(
			NSAutoresizingMaskOptions::ViewWidthSizable
				| NSAutoresizingMaskOptions::ViewHeightSizable,
		);
		view.setHidden(true);
		frame.addSubview_positioned_relativeTo(&view, NSWindowOrderingMode::Below, Some(content));
		Some(Self { view })
	}

	pub fn set_enabled(&self, enabled: bool) {
		self.view.setHidden(!enabled);
	}
}

impl Drop for Blur {
	fn drop(&mut self) {
		self.view.removeFromSuperview();
	}
}
