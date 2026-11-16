use super::{Event, Events, MenuState, Row};
use objc2::{
	AllocAnyThread, DefinedClass, MainThreadOnly, define_class, msg_send, rc::Retained, sel,
};
use objc2_app_kit::{NSImage, NSMenu, NSMenuItem, NSStatusBar, NSStatusItem};
use objc2_foundation::{
	MainThreadMarker, NSData, NSObject, NSObjectProtocol, NSSize, NSString, ns_string,
};
use std::{cell::RefCell, sync::Arc};

/// Serein's own mark, rasterized from `assets/brand/serein-mark.svg` as a menu bar template:
/// only its alpha matters, so macOS tints it for light, dark and highlighted menu bars.
const MARK: &[u8] = include_bytes!("../../../../assets/brand/serein-tray.png");
/// Menu bar images are measured in points; the render is larger so Retina scales stay sharp.
const MARK_POINTS: f64 = 15.0;

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

		#[unsafe(method(toggleMute:))]
		fn toggle_mute(&self, _sender: &NSMenuItem) {
			self.emit(Event::ToggleMute);
		}

		#[unsafe(method(toggleDeafen:))]
		fn toggle_deafen(&self, _sender: &NSMenuItem) {
			self.emit(Event::ToggleDeafen);
		}

		#[unsafe(method(disconnectCall:))]
		fn disconnect(&self, _sender: &NSMenuItem) {
			self.emit(Event::Disconnect);
		}
	}
);

impl Target {
	fn emit(&self, event: Event) {
		let state = self.ivars();
		// Restore before Quit as well, so the app can display its unsaved-draft prompt.
		if event.restores() {
			state.window.set_visible(true);
			state.window.set_minimized(false);
			state.window.focus_window();
		}
		state.events.push(event);
		(state.wake)();
	}
}

/// Main-thread ownership prevents cross-thread callbacks and destruction.
pub struct Tray {
	item: Retained<NSStatusItem>,
	menu: Retained<NSMenu>,
	target: Retained<Target>,
	shown: RefCell<MenuState>,
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
			shown: RefCell::default(),
		};
		let button = tray
			.item
			.button(mtm)
			.ok_or("The macOS menu bar is unavailable.")?;
		let image = NSImage::initWithData(NSImage::alloc(), &NSData::with_bytes(MARK));
		if let Some(image) = image {
			image.setTemplate(true);
			image.setSize(NSSize::new(MARK_POINTS, MARK_POINTS));
			button.setImage(Some(&image));
		} else {
			button.setTitle(ns_string!("Serein"));
		}
		button.setToolTip(Some(ns_string!("Serein")));
		tray.fill(mtm);
		tray.item.setMenu(Some(&tray.menu));
		Ok(tray)
	}

	/// Replaces the menu's items with the current rows.
	fn fill(&self, mtm: MainThreadMarker) {
		self.clear();
		for row in self.shown.borrow().rows() {
			let (title, action, checked, enabled) = match row {
				Row::Separator => {
					self.menu.addItem(&NSMenuItem::separatorItem(mtm));
					continue;
				}
				Row::Info(text) => (text, None, None, false),
				Row::Action {
					label,
					event,
					checked,
					enabled,
				} => {
					let action = match event {
						Event::Show => sel!(showSerein:),
						Event::Quit => sel!(quitSerein:),
						Event::ToggleMute => sel!(toggleMute:),
						Event::ToggleDeafen => sel!(toggleDeafen:),
						Event::Disconnect => sel!(disconnectCall:),
						Event::Unavailable => continue,
					};
					(label.to_owned(), Some(action), checked, enabled)
				}
			};
			// SAFETY: every selector is implemented above with the menu action signature.
			// Tray retains their main-thread target until the menu is disconnected.
			let item = unsafe {
				let item = NSMenuItem::initWithTitle_action_keyEquivalent(
					NSMenuItem::alloc(mtm),
					&NSString::from_str(&title),
					action,
					ns_string!(""),
				);
				if action.is_some() {
					item.setTarget(Some(&self.target));
				}
				if checked == Some(true) {
					// NSControlStateValueOn, without the NSCell bindings it is declared in.
					let _: () = msg_send![&*item, setState: 1isize];
				}
				item
			};
			item.setEnabled(enabled);
			self.menu.addItem(&item);
		}
	}

	fn clear(&self) {
		for item in self.menu.itemArray() {
			// SAFETY: disconnect even a menu item retained by AppKit's active menu tracking.
			unsafe { item.setTarget(None) };
		}
		self.menu.removeAllItems();
	}

	pub fn set_menu(&self, menu: &super::Menu<'_>) {
		if self.shown.borrow().matches(menu) {
			return;
		}
		*self.shown.borrow_mut() = MenuState::new(menu);
		if let Some(mtm) = MainThreadMarker::new() {
			self.fill(mtm);
		}
	}

	pub fn take_event(&self) -> Option<Event> {
		self.target.ivars().events.take()
	}

	pub fn set_voice_state(&self, _state: super::VoiceState) {}
}

impl Drop for Tray {
	fn drop(&mut self) {
		self.item.setMenu(None);
		self.clear();
		NSStatusBar::systemStatusBar().removeStatusItem(&self.item);
	}
}
