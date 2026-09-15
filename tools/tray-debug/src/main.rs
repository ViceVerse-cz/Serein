//! Synthetic tray checks: private `dbus-run-session` on Linux, native main-thread window on macOS.
#[cfg(any(target_os = "linux", target_os = "macos"))]
#[path = "../../../crates/platform/src/tray.rs"]
mod tray;
#[cfg(all(target_os = "linux", test))]
pub use egui;
#[cfg(all(target_os = "linux", test))]
#[path = "../../../apps/desktop/src/tray_window.rs"]
mod tray_window;

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn main() {
	eprintln!("Run on Linux inside dbus-run-session, or on macOS from a graphical session.");
}

#[cfg(target_os = "linux")]
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
	tokio::time::timeout(std::time::Duration::from_secs(15), linux::check()).await?
}

#[cfg(target_os = "linux")]
mod linux {
	use super::tray::{self, Event, Tray};
	use std::{
		sync::{
			Arc,
			atomic::{AtomicBool, Ordering},
		},
		time::Duration,
	};
	use tokio::sync::{Notify, watch};

	struct Watcher(watch::Sender<String>, Arc<AtomicBool>);
	#[zbus::interface(name = "org.kde.StatusNotifierWatcher")]
	impl Watcher {
		fn register_status_notifier_item(&self, service: String) {
			self.0.send_replace(service);
		}
		#[zbus(property)]
		async fn is_status_notifier_host_registered(
			&self,
			#[zbus(connection)] bus: &zbus::Connection,
		) -> bool {
			if self.1.swap(false, Ordering::Relaxed) {
				bus.release_name("org.kde.StatusNotifierWatcher")
					.await
					.unwrap();
			}
			true
		}
	}

	async fn event(tray: &Tray, wake: &Notify, expected: Event) {
		loop {
			if let Some(actual) = tray.take_event() {
				assert_eq!(actual, expected);
				return;
			}
			wake.notified().await;
		}
	}

	pub async fn check() -> Result<(), Box<dyn std::error::Error>> {
		assert!(tray::supported());
		let bus = zbus::Connection::session().await?;
		let dbus = zbus::fdo::DBusProxy::new(&bus).await?;
		assert!(
			!dbus
				.name_has_owner("org.kde.StatusNotifierWatcher".try_into()?)
				.await?,
			"Use dbus-run-session; never replace a real desktop watcher."
		);
		let wake = Arc::new(Notify::new());
		let start = || {
			let wake = wake.clone();
			Tray::new(move || wake.notify_one()).unwrap()
		};
		let missing = start();
		event(&missing, &wake, Event::Unavailable).await;
		assert!(!missing.is_available());
		drop(missing);
		println!("PASS: absent host");

		let (registered, mut registration) = watch::channel(String::new());
		let lose_during_registration = Arc::new(AtomicBool::new(false));
		let host = zbus::connection::Builder::session()?
			.name("org.kde.StatusNotifierWatcher")?
			.serve_at(
				"/StatusNotifierWatcher",
				Watcher(registered, lose_during_registration.clone()),
			)?
			.build()
			.await?;
		let tray = start();
		registration.changed().await?;
		while !tray.is_available() {
			wake.notified().await;
		}
		println!("PASS: registration");
		let service = registration.borrow_and_update().clone();
		let item = zbus::Proxy::new(
			&bus,
			service.as_str(),
			"/StatusNotifierItem",
			"org.kde.StatusNotifierItem",
		)
		.await?;
		assert_eq!(item.get_property::<String>("Title").await?, "Serein");
		let icon: Vec<(i32, i32, Vec<u8>)> = item.get_property("IconPixmap").await?;
		assert_eq!(icon.len(), 1);
		assert_eq!((icon[0].0, icon[0].1, icon[0].2.len()), (32, 32, 4096));
		for _ in 0..16 {
			item.call::<_, _, ()>("Activate", &(0i32, 0i32)).await?;
		}
		event(&tray, &wake, Event::Show).await;
		assert_eq!(tray.take_event(), None, "Show events must coalesce");
		println!("PASS: icon and coalesced activation");
		let menu_path: zbus::zvariant::OwnedObjectPath = item.get_property("Menu").await?;
		let menu =
			zbus::Proxy::new(&bus, service.clone(), menu_path, "com.canonical.dbusmenu").await?;
		for (id, expected) in [(1i32, Event::Show), (2, Event::Quit)] {
			menu.call::<_, _, ()>(
				"Event",
				&(id, "clicked", zbus::zvariant::Value::new(0i32), 0u32),
			)
			.await?;
			event(&tray, &wake, expected).await;
			println!("PASS: menu {expected:?}");
		}
		host.release_name("org.kde.StatusNotifierWatcher").await?;
		event(&tray, &wake, Event::Unavailable).await;
		assert!(!tray.is_available());
		drop(tray);
		println!("PASS: host loss");
		while dbus.name_has_owner(service.as_str().try_into()?).await? {
			tokio::time::sleep(Duration::from_millis(10)).await;
		}
		host.request_name("org.kde.StatusNotifierWatcher").await?;
		let tray = start();
		registration.changed().await?;
		let service = registration.borrow_and_update().clone();
		let item = zbus::Proxy::new(
			&bus,
			service.as_str(),
			"/StatusNotifierItem",
			"org.kde.StatusNotifierItem",
		)
		.await?;
		item.call::<_, _, ()>("Activate", &(0i32, 0i32)).await?;
		event(&tray, &wake, Event::Show).await;
		drop(tray);
		while dbus.name_has_owner(service.as_str().try_into()?).await? {
			tokio::time::sleep(Duration::from_millis(10)).await;
		}
		lose_during_registration.store(true, Ordering::Relaxed);
		let tray = start();
		event(&tray, &wake, Event::Unavailable).await;
		assert!(!tray.is_available());
		drop(tray);
		println!("PASS: host lost during registration");
		println!(
			"PASS: absent host, registration, icon, coalesced Show, menu Show/Quit, unregister and host loss (synthetic private D-Bus; no desktop rendering)."
		);
		Ok(())
	}
}

#[cfg(target_os = "macos")]
#[allow(deprecated)]
fn main() {
	use objc2::{MainThreadMarker, msg_send, runtime::AnyObject};
	use objc2_app_kit::{NSApplication, NSApplicationTerminateReply as Reply};
	use std::sync::Arc;
	assert!(tray::supported());
	let event_loop = winit::event_loop::EventLoop::new().unwrap();
	let window = Arc::new(
		event_loop
			.create_window(
				winit::window::Window::default_attributes()
					.with_title("Serein synthetic tray check")
					.with_visible(false),
			)
			.unwrap(),
	);
	let app = NSApplication::sharedApplication(MainThreadMarker::new().unwrap());
	let delegate = app.delegate().unwrap();
	let original: &AnyObject = (*delegate).as_ref();
	let request = || -> Reply {
		// SAFETY: Tray installs the protocol callback with this exact signature.
		unsafe { msg_send![&delegate, applicationShouldTerminate: &*app] }
	};
	for _ in 0..2 {
		let tray = tray::Tray::new(window.clone(), || {}).unwrap();
		assert!(tray.is_available());
		let current = app.delegate().unwrap();
		let current: &AnyObject = (*current).as_ref();
		assert!(
			std::ptr::eq(original, current),
			"winit delegate identity must survive"
		);
		assert!(tray::Tray::new(window.clone(), || {}).is_err());
		assert_eq!(request(), Reply::TerminateCancel);
		assert_eq!(tray.take_event(), Some(tray::Event::Close));
		drop(tray);
		assert_eq!(request(), Reply::TerminateNow);
	}
	println!(
		"PASS: macOS termination cancellation, delegate identity, duplicate rejection, disable and re-enable (synthetic window; no account)."
	);
}
