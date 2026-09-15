use super::{Event, Events};
use objc2::{DefinedClass, MainThreadOnly, define_class, msg_send, rc::Retained, sel};
use objc2_app_kit::{NSImage, NSMenu, NSMenuItem, NSStatusBar, NSStatusItem};
use objc2_foundation::{MainThreadMarker, NSObject, NSObjectProtocol, ns_string};
use std::sync::Arc;

struct State {
	window: Arc<winit::window::Window>,
	events: Events,
	wake: Box<dyn Fn()>,
}

define_class!(
	// SAFETY: NSObject has no subclassing requirements. All state and callbacks stay on
	// the main thread, and the target lives until every menu item is disconnected.
	#[unsafe(super = NSObject)]
	#[name = "SereinTrayTarget"]
	#[thread_kind = MainThreadOnly]
	#[ivars = State]
	struct Target;

	// SAFETY: NSObjectProtocol has no additional requirements.
	unsafe impl NSObjectProtocol for Target {}

	impl Target {
		#[unsafe(method(showSerein:))]
		fn show(&self, _sender: &NSMenuItem) {
			self.emit(Event::Show);
		}

		#[unsafe(method(quitSerein:))]
		fn quit(&self, _sender: &NSMenuItem) {
			self.emit(Event::Quit);
		}
	}
);

impl Target {
	fn emit(&self, event: Event) {
		let state = self.ivars();
		// Restore before Quit as well, so the app can display its unsaved-draft prompt.
		state.window.set_visible(true);
		state.window.set_minimized(false);
		state.window.focus_window();
		state.events.push(event);
		(state.wake)();
	}
}

/// Main-thread ownership prevents cross-thread callbacks and destruction.
pub struct Tray {
	item: Retained<NSStatusItem>,
	menu: Retained<NSMenu>,
	target: Retained<Target>,
}

impl Tray {
	pub fn new(
		window: Arc<winit::window::Window>,
		wake: impl Fn() + 'static,
	) -> Result<Self, &'static str> {
		let mtm = MainThreadMarker::new().ok_or("The menu bar icon requires the main thread.")?;
		let target = Target::alloc(mtm).set_ivars(State {
			window,
			events: Events::default(),
			wake: Box::new(wake),
		});
		// SAFETY: NSObject init initializes this allocated NSObject subclass.
		let target = unsafe { msg_send![super(target), init] };
		let menu = NSMenu::new(mtm);
		menu.setAutoenablesItems(false);
		let tray = Self {
			// NSVariableStatusItemLength: fit the symbol or fallback title.
			item: NSStatusBar::systemStatusBar().statusItemWithLength(-1.0),
			menu,
			target,
		};
		let button = tray
			.item
			.button(mtm)
			.ok_or("The macOS menu bar is unavailable.")?;
		let image = if objc2::available!(macos = 11.0) {
			NSImage::imageWithSystemSymbolName_accessibilityDescription(
				ns_string!("bubble.left.and.bubble.right"),
				Some(ns_string!("Serein")),
			)
		} else {
			None
		};
		if let Some(image) = image {
			image.setTemplate(true);
			button.setImage(Some(&image));
		} else {
			button.setTitle(ns_string!("Serein"));
		}
		button.setToolTip(Some(ns_string!("Serein")));
		for (title, action) in [
			(ns_string!("Show Serein"), sel!(showSerein:)),
			(ns_string!("Quit Serein"), sel!(quitSerein:)),
		] {
			// SAFETY: both selectors are implemented above with the menu action signature.
			// Tray retains their main-thread target until the menu is disconnected on drop.
			let item = unsafe {
				let item = NSMenuItem::initWithTitle_action_keyEquivalent(
					NSMenuItem::alloc(mtm),
					title,
					Some(action),
					ns_string!(""),
				);
				item.setTarget(Some(&tray.target));
				item
			};
			tray.menu.addItem(&item);
		}
		tray.item.setMenu(Some(&tray.menu));
		Ok(tray)
	}

	pub fn take_event(&self) -> Option<Event> {
		self.target.ivars().events.take()
	}
}

impl Drop for Tray {
	fn drop(&mut self) {
		self.item.setMenu(None);
		for item in self.menu.itemArray() {
			// SAFETY: disconnect even a menu item retained by AppKit's active menu tracking.
			unsafe { item.setTarget(None) };
		}
		self.menu.removeAllItems();
		NSStatusBar::systemStatusBar().removeStatusItem(&self.item);
	}
}
