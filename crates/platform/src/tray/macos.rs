use super::{Event, Events};
use objc2::{
	DefinedClass, MainThreadOnly, define_class, msg_send,
	rc::Retained,
	runtime::{AnyObject, Imp, Sel},
	sel,
};
use objc2_app_kit::{
	NSApplication, NSApplicationTerminateReply, NSImage, NSMenu, NSMenuItem, NSStatusBar,
	NSStatusItem,
};
use objc2_foundation::{MainThreadMarker, NSObject, NSObjectProtocol, ns_string};
use std::{cell::RefCell, sync::Arc};

thread_local! {
	// Exactly one UI-thread-owned tray can intercept application termination.
	static TERMINATION_TARGET: RefCell<Option<Retained<Target>>> = const { RefCell::new(None) };
}

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
		if event != Event::Close {
			state.window.set_visible(true);
			state.window.set_minimized(false);
			state.window.focus_window();
		}
		state.events.push(event);
		(state.wake)();
	}
}

unsafe extern "C-unwind" fn should_terminate(
	_delegate: &AnyObject,
	_selector: Sel,
	_application: &NSApplication,
) -> NSApplicationTerminateReply {
	// Release the RefCell borrow before waking the event loop, which may reenter AppKit.
	let target = TERMINATION_TARGET.with(|slot| slot.borrow().clone());
	if let Some(target) = target {
		target.emit(Event::Close);
		NSApplicationTerminateReply::TerminateCancel
	} else {
		// AppKit's default for a delegate without applicationShouldTerminate:.
		NSApplicationTerminateReply::TerminateNow
	}
}

fn intercept_termination(mtm: MainThreadMarker) -> Result<(), &'static str> {
	if TERMINATION_TARGET.with(|slot| slot.borrow().is_some()) {
		return Err("A macOS tray icon is already active.");
	}
	let delegate = NSApplication::sharedApplication(mtm)
		.delegate()
		.ok_or("The macOS application delegate is unavailable.")?;
	let object: &AnyObject = (*delegate).as_ref();
	let class = object.class();
	if class.name() != c"WinitApplicationDelegate" {
		return Err("The macOS application delegate cannot support close to tray.");
	}
	let selector = sel!(applicationShouldTerminate:);
	// SAFETY: Objective-C IMP erases the typed signature. Our callback matches the
	// protocol: NSUInteger return, object receiver, selector, NSApplication argument.
	let implementation: Imp = unsafe {
		std::mem::transmute(
			should_terminate
				as unsafe extern "C-unwind" fn(
					&AnyObject,
					Sel,
					&NSApplication,
				) -> NSApplicationTerminateReply,
		)
	};
	if let Some(method) = class.instance_method(selector) {
		if !std::ptr::fn_addr_eq(method.implementation(), implementation) {
			return Err("The macOS application already has a termination handler.");
		}
	} else {
		// SAFETY: add only a missing optional method to the existing delegate class;
		// never replace the delegate or its methods/ivars, which winit relies on.
		// Q@:@ is NSUInteger/object/selector/object on supported 64-bit macOS targets.
		// This process-lifetime callback owns no raw state and permits exit when inactive.
		let added = unsafe {
			objc2::ffi::class_addMethod(
				(class as *const objc2::runtime::AnyClass).cast_mut(),
				selector,
				implementation,
				c"Q@:@".as_ptr(),
			)
		};
		if !added.as_bool() {
			return Err("The macOS termination handler could not be installed.");
		}
		// Re-register the same delegate so AppKit can refresh optional callbacks.
		NSApplication::sharedApplication(mtm).setDelegate(Some(&delegate));
	}
	Ok(())
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
		intercept_termination(mtm)?;
		TERMINATION_TARGET.with(|slot| *slot.borrow_mut() = Some(tray.target.clone()));
		Ok(tray)
	}

	pub fn is_available(&self) -> bool {
		self.item.isVisible()
	}

	pub fn take_event(&self) -> Option<Event> {
		self.target.ivars().events.take()
	}
}

impl Drop for Tray {
	fn drop(&mut self) {
		TERMINATION_TARGET.with(|slot| {
			let mut target = slot.borrow_mut();
			if target.as_ref().is_some_and(|target| {
				std::ptr::eq(Retained::as_ptr(target), Retained::as_ptr(&self.target))
			}) {
				target.take()
			} else {
				None
			}
		});
		self.item.setMenu(None);
		for item in self.menu.itemArray() {
			// SAFETY: disconnect even a menu item retained by AppKit's active menu tracking.
			unsafe { item.setTarget(None) };
		}
		self.menu.removeAllItems();
		NSStatusBar::systemStatusBar().removeStatusItem(&self.item);
	}
}
