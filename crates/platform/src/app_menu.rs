//! Native macOS menu bar. Shortcuts egui already reads (editing, zoom) stay with the focused
//! view, so typing behaves as before and the menu only routes clicks. Webviews such as the
//! login page receive the standard editing actions through the responder chain instead.
#![allow(unsafe_code)]

use objc2::{
	DefinedClass, MainThreadOnly, define_class, msg_send,
	rc::Retained,
	runtime::{AnyObject, Sel},
	sel,
};
use objc2_app_kit::{
	NSApplication, NSEventModifierFlags, NSEventType, NSMenu, NSMenuItem, NSView, NSWindow,
};
use objc2_foundation::{MainThreadMarker, NSObject, NSObjectProtocol, NSString, ns_string};
use std::{cell::RefCell, collections::VecDeque, sync::Arc};

/// Menu requests the application handles. The menu keeps at most a few per frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
	Settings,
	Updates,
	Quit,
	CloseWindow,
	Undo,
	Redo,
	Cut,
	Copy,
	Paste,
	SelectAll,
	ZoomIn,
	ZoomOut,
	ActualSize,
	Website,
	ReportIssue,
}

const COMMANDS: [Command; 15] = [
	Command::Settings,
	Command::Updates,
	Command::Quit,
	Command::CloseWindow,
	Command::Undo,
	Command::Redo,
	Command::Cut,
	Command::Copy,
	Command::Paste,
	Command::SelectAll,
	Command::ZoomIn,
	Command::ZoomOut,
	Command::ActualSize,
	Command::Website,
	Command::ReportIssue,
];
const QUEUE_LIMIT: usize = 8;

impl Command {
	fn tag(self) -> isize {
		COMMANDS
			.iter()
			.position(|command| *command == self)
			.unwrap_or(0) as isize
	}
	fn from_tag(tag: isize) -> Option<Self> {
		COMMANDS.get(usize::try_from(tag).ok()?).copied()
	}
	/// egui reads these shortcuts itself; a menu key equivalent would swallow the key.
	fn typed_in_view(self) -> bool {
		matches!(
			self,
			Self::Undo
				| Self::Redo | Self::Cut
				| Self::Copy | Self::Paste
				| Self::SelectAll
				| Self::ZoomIn
				| Self::ZoomOut
				| Self::ActualSize
		)
	}
	/// Standard editing action, for native views such as the login webview.
	fn responder_action(self) -> Option<Sel> {
		Some(match self {
			Self::Undo => sel!(undo:),
			Self::Redo => sel!(redo:),
			Self::Cut => sel!(cut:),
			Self::Copy => sel!(copy:),
			Self::Paste => sel!(paste:),
			Self::SelectAll => sel!(selectAll:),
			_ => return None,
		})
	}
	/// The window may be hidden while the app stays active, and these must show it.
	fn shows_window(self) -> bool {
		matches!(self, Self::Settings | Self::Updates | Self::Quit)
	}
}

struct State {
	window: Arc<winit::window::Window>,
	ns_window: Retained<NSWindow>,
	view: Retained<NSView>,
	commands: RefCell<VecDeque<Command>>,
	wake: Box<dyn Fn()>,
}

define_class!(
	// SAFETY: NSObject has no subclassing requirements. All state and callbacks stay on
	// the main thread, and the target lives until every menu item is disconnected.
	#[unsafe(super = NSObject)]
	#[name = "SereinMenuTarget"]
	#[thread_kind = MainThreadOnly]
	#[ivars = State]
	struct Target;

	// SAFETY: NSObjectProtocol has no additional requirements.
	unsafe impl NSObjectProtocol for Target {}

	impl Target {
		#[unsafe(method(sereinCommand:))]
		fn command(&self, item: &NSMenuItem) {
			if let Some(command) = Command::from_tag(item.tag()) {
				self.run(command, item);
			}
		}

		#[unsafe(method(validateMenuItem:))]
		fn validate(&self, item: &NSMenuItem) -> bool {
			Command::from_tag(item.tag()).is_some_and(|command| self.enabled(command, item))
		}
	}
);

impl Target {
	fn enabled(&self, command: Command, item: &NSMenuItem) -> bool {
		let state = self.ivars();
		let app = NSApplication::sharedApplication(self.mtm());
		if self.responder(command, item) {
			return true;
		}
		if command.typed_in_view() {
			// Disabled items let the key event continue to egui, as without a menu.
			return state.ns_window.isVisible() && !key_equivalent(&app);
		}
		command != Command::CloseWindow || app.keyWindow().is_some()
	}

	/// A native view other than egui's that handles the standard editing action.
	fn responder(&self, command: Command, item: &NSMenuItem) -> bool {
		let Some(action) = command.responder_action() else {
			return false;
		};
		let app = NSApplication::sharedApplication(self.mtm());
		// SAFETY: a standard editing selector, sent from a menu item like AppKit's own.
		let target = unsafe { app.targetForAction_to_from(action, None, Some(item)) };
		target
			.as_deref()
			.and_then(AnyObject::downcast_ref::<NSView>)
			.is_some_and(|view| !std::ptr::eq(view, &*self.ivars().view))
	}

	fn run(&self, command: Command, item: &NSMenuItem) {
		let state = self.ivars();
		let app = NSApplication::sharedApplication(self.mtm());
		if self.responder(command, item)
			&& let Some(action) = command.responder_action()
		{
			// SAFETY: as in `responder`; the first responder implements this action.
			unsafe { app.sendAction_to_from(action, None, Some(item)) };
			return;
		}
		if command == Command::CloseWindow
			&& let Some(key) = app.keyWindow()
			&& !std::ptr::eq(&*key, &*state.ns_window)
		{
			key.performClose(Some(item));
			return;
		}
		if command.shows_window() {
			state.window.set_visible(true);
			state.window.set_minimized(false);
			state.window.focus_window();
		}
		let mut commands = state.commands.borrow_mut();
		if commands.len() < QUEUE_LIMIT {
			commands.push_back(command);
		}
		drop(commands);
		(state.wake)();
	}
}

/// The menu is validating a key equivalent rather than opening for a click.
fn key_equivalent(app: &NSApplication) -> bool {
	app.currentEvent().is_some_and(|event| {
		event.r#type() == NSEventType::KeyDown
			&& event
				.modifierFlags()
				.contains(NSEventModifierFlags::Command)
	})
}

/// A retitled item, its submenu (whose title the menu bar shows) and its localization key.
type Title = (Retained<NSMenuItem>, Option<Retained<NSMenu>>, &'static str);

/// Main-thread ownership prevents cross-thread callbacks and destruction.
pub struct AppMenu {
	bar: Retained<NSMenu>,
	target: Retained<Target>,
	titles: Vec<Title>,
}

impl AppMenu {
	/// Replaces winit's minimal menu. `translate` looks up the `app-menu-*` keys.
	pub fn new(
		window: Arc<winit::window::Window>,
		wake: impl Fn() + 'static,
		translate: impl Fn(&str) -> String,
	) -> Result<Self, &'static str> {
		use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};

		let mtm = MainThreadMarker::new().ok_or("The menu bar requires the main thread.")?;
		let RawWindowHandle::AppKit(handle) = window
			.window_handle()
			.map_err(|_| "Native window unavailable.")?
			.as_raw()
		else {
			return Err("Expected a macOS window.");
		};
		// SAFETY: winit owns this NSView for the window's lifetime, which the retained
		// `window` outlives; retaining the view and its window keeps both valid.
		let view: Retained<NSView> =
			unsafe { Retained::retain(handle.ns_view.cast::<NSView>().as_ptr()) }
				.ok_or("Native window unavailable.")?;
		let ns_window = view.window().ok_or("Native window unavailable.")?;
		let target = Target::alloc(mtm).set_ivars(State {
			window,
			ns_window,
			view,
			commands: RefCell::default(),
			wake: Box::new(wake),
		});
		// SAFETY: NSObject init initializes this allocated NSObject subclass.
		let target = unsafe { msg_send![super(target), init] };
		let mut menu = Self {
			bar: NSMenu::new(mtm),
			target,
			titles: Vec::new(),
		};
		menu.build(mtm);
		menu.translate(translate);
		NSApplication::sharedApplication(mtm).setMainMenu(Some(&menu.bar));
		Ok(menu)
	}

	fn build(&mut self, mtm: MainThreadMarker) {
		use Command::*;
		const SHIFT: NSEventModifierFlags = NSEventModifierFlags::Shift;
		const OPTION: NSEventModifierFlags = NSEventModifierFlags::Option;
		const CONTROL: NSEventModifierFlags = NSEventModifierFlags::Control;
		let app = NSApplication::sharedApplication(mtm);

		// macOS titles the application menu with the process name in bold.
		let menu = self.submenu(mtm, None);
		self.native(
			&menu,
			"app-menu-about",
			sel!(orderFrontStandardAboutPanel:),
			"",
			None,
		);
		menu.addItem(&NSMenuItem::separatorItem(mtm));
		self.command(&menu, "app-menu-updates", Updates, "", None);
		self.command(&menu, "app-menu-settings", Settings, ",", None);
		menu.addItem(&NSMenuItem::separatorItem(mtm));
		let services = NSMenu::new(mtm);
		let item = self.native(&menu, "app-menu-services", None, "", None);
		item.setSubmenu(Some(&services));
		app.setServicesMenu(Some(&services));
		menu.addItem(&NSMenuItem::separatorItem(mtm));
		self.native(&menu, "app-menu-hide", sel!(hide:), "h", None);
		self.native(
			&menu,
			"app-menu-hide-others",
			sel!(hideOtherApplications:),
			"h",
			Some(OPTION),
		);
		self.native(
			&menu,
			"app-menu-show-all",
			sel!(unhideAllApplications:),
			"",
			None,
		);
		menu.addItem(&NSMenuItem::separatorItem(mtm));
		self.command(&menu, "app-menu-quit", Quit, "q", None);

		let menu = self.submenu(mtm, Some("app-menu-file"));
		self.command(&menu, "app-menu-close-window", CloseWindow, "w", None);

		let menu = self.submenu(mtm, Some("app-menu-edit"));
		self.command(&menu, "app-menu-undo", Undo, "z", None);
		self.command(&menu, "app-menu-redo", Redo, "z", Some(SHIFT));
		menu.addItem(&NSMenuItem::separatorItem(mtm));
		self.command(&menu, "app-menu-cut", Cut, "x", None);
		self.command(&menu, "app-menu-copy", Copy, "c", None);
		self.command(&menu, "app-menu-paste", Paste, "v", None);
		self.command(&menu, "app-menu-select-all", SelectAll, "a", None);
		menu.addItem(&NSMenuItem::separatorItem(mtm));
		self.native(
			&menu,
			"app-menu-emoji",
			sel!(orderFrontCharacterPalette:),
			" ",
			Some(CONTROL),
		);

		let menu = self.submenu(mtm, Some("app-menu-view"));
		self.command(&menu, "app-menu-actual-size", ActualSize, "0", None);
		self.command(&menu, "app-menu-zoom-in", ZoomIn, "+", None);
		self.command(&menu, "app-menu-zoom-out", ZoomOut, "-", None);
		menu.addItem(&NSMenuItem::separatorItem(mtm));
		self.native(
			&menu,
			"app-menu-full-screen",
			sel!(toggleFullScreen:),
			"f",
			Some(CONTROL),
		);

		let menu = self.submenu(mtm, Some("app-menu-window"));
		self.native(
			&menu,
			"app-menu-minimize",
			sel!(performMiniaturize:),
			"m",
			None,
		);
		self.native(&menu, "app-menu-zoom", sel!(performZoom:), "", None);
		menu.addItem(&NSMenuItem::separatorItem(mtm));
		self.native(
			&menu,
			"app-menu-bring-all-to-front",
			sel!(arrangeInFront:),
			"",
			None,
		);
		app.setWindowsMenu(Some(&menu));

		let menu = self.submenu(mtm, Some("app-menu-help"));
		self.command(&menu, "app-menu-github", Website, "", None);
		self.command(&menu, "app-menu-report-issue", ReportIssue, "", None);
		app.setHelpMenu(Some(&menu));
	}

	fn submenu(&mut self, mtm: MainThreadMarker, title: Option<&'static str>) -> Retained<NSMenu> {
		let item = NSMenuItem::new(mtm);
		let menu = NSMenu::new(mtm);
		item.setSubmenu(Some(&menu));
		self.bar.addItem(&item);
		if let Some(title) = title {
			self.titles.push((item, Some(menu.clone()), title));
		}
		menu
	}

	/// An AppKit action sent through the responder chain.
	fn native(
		&mut self,
		menu: &NSMenu,
		title: &'static str,
		action: impl Into<Option<Sel>>,
		key: &str,
		modifiers: Option<NSEventModifierFlags>,
	) -> Retained<NSMenuItem> {
		let mtm = menu.mtm();
		// SAFETY: standard AppKit actions with the menu action signature, or none.
		let item = unsafe {
			NSMenuItem::initWithTitle_action_keyEquivalent(
				NSMenuItem::alloc(mtm),
				ns_string!(""),
				action.into(),
				&NSString::from_str(key),
			)
		};
		if let Some(modifiers) = modifiers {
			item.setKeyEquivalentModifierMask(modifiers | NSEventModifierFlags::Command);
		}
		menu.addItem(&item);
		self.titles.push((item.clone(), None, title));
		item
	}

	fn command(
		&mut self,
		menu: &NSMenu,
		title: &'static str,
		command: Command,
		key: &str,
		modifiers: Option<NSEventModifierFlags>,
	) {
		let item = self.native(menu, title, sel!(sereinCommand:), key, modifiers);
		item.setTag(command.tag());
		// SAFETY: the target implements `sereinCommand:` and outlives the item's connection.
		unsafe { item.setTarget(Some(&self.target)) };
	}

	/// Retitles every item, for example after the interface language changes.
	pub fn translate(&self, translate: impl Fn(&str) -> String) {
		for (item, menu, key) in &self.titles {
			let title = NSString::from_str(&translate(key));
			item.setTitle(&title);
			if let Some(menu) = menu {
				menu.setTitle(&title);
			}
		}
	}

	pub fn take_command(&self) -> Option<Command> {
		self.target.ivars().commands.borrow_mut().pop_front()
	}
}

impl Drop for AppMenu {
	fn drop(&mut self) {
		for (item, _, _) in &self.titles {
			// SAFETY: disconnect even a menu item retained by AppKit's active menu tracking.
			unsafe { item.setTarget(None) };
		}
	}
}
