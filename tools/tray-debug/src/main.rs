//! Offline Linux tray check. Run with `dbus-run-session -- cargo run --locked -p tray-debug`.
#[cfg(target_os = "linux")]
#[path = "../../../crates/platform/src/tray.rs"]
mod tray;

pub use egui;
// `tray_window.rs` addresses the compositor helper through `platform::compositor`, so this
// tool depends on the real crate; it's already built for other workspace members.
use platform as _;
#[path = "../../../apps/desktop/src/tray_window.rs"]
mod tray_window;

#[cfg(not(target_os = "linux"))]
fn main() {
	check_window();
	println!("Linux D-Bus and compositor checks require Linux; not run on this host.");
}

#[cfg(target_os = "linux")]
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
	check_window();
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
			Tray::new(move || wake.notify_one(), || {}).unwrap()
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
		for state in [
			tray::VoiceState::Unmuted,
			tray::VoiceState::Speaking,
			tray::VoiceState::Muted,
			tray::VoiceState::Deafened,
		] {
			tray.set_voice_state(state);
		}
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
		for (id, expected) in [(1i32, Event::Show), (2, Event::Minimize), (3, Event::Quit)] {
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
			"PASS: absent host, registration, icon, coalesced Show, menu Show/Minimize/Quit, unregister and host loss (synthetic private D-Bus; no desktop rendering)."
		);
		Ok(())
	}
}

/// Exercise the desktop's actual close policy without creating a window or account.
fn check_window() {
	use egui::{ViewportCommand as Cmd, ViewportId};
	let ctx = egui::Context::default();
	let mut state = tray_window::State::default();
	let _restore = state.restorer();
	let mut close = egui::RawInput::default();
	close
		.viewports
		.get_mut(&ViewportId::ROOT)
		.unwrap()
		.events
		.push(egui::ViewportEvent::Close);
	for available in [false, true] {
		let _ = ctx.run_logic(&close, |ctx| {
			state.logic(ctx, available, true, Some(true));
			assert!(ctx.input(|i| i.viewport().close_requested()));
		});
	}
	state.cancel_quit();
	for can_hide in [true, false] {
		// Each case starts from a mapped window.
		state.show(&ctx);
		let output = ctx.run_logic(&close, |ctx| {
			state.logic(ctx, true, can_hide, Some(true));
			assert!(!ctx.input(|i| i.viewport().close_requested()));
			assert!(
				!state.hidden,
				"the hide stays pending until it is confirmed"
			);
		});
		let cmds = &output.viewport_commands[&ViewportId::ROOT];
		assert!(cmds.contains(&Cmd::CancelClose));
		assert!(cmds.contains(&if can_hide {
			Cmd::Visible(false)
		} else {
			Cmd::Minimized(true)
		}));
		// The compositor applies the request: unmapped for a winit hide, minimized otherwise.
		let mut confirmed = egui::RawInput::default();
		if !can_hide {
			confirmed
				.viewports
				.get_mut(&ViewportId::ROOT)
				.unwrap()
				.minimized = Some(true);
		}
		let output = ctx.run_logic(&confirmed, |ctx| {
			state.logic(ctx, true, can_hide, can_hide.then_some(false));
		});
		assert!(
			state.hidden,
			"a confirmed hide parks the window in the tray"
		);
		assert!(
			output
				.viewport_commands
				.get(&ViewportId::ROOT)
				.is_none_or(|cmds| !cmds.contains(&Cmd::Close)),
			"a confirmed hide must not also close"
		);
	}
	// Nobody can close the hidden window, so this close is the session ending: exit, unconfirmed.
	let output = ctx.run_logic(&close, |ctx| state.logic(ctx, true, true, Some(false)));
	let cmds = &output.viewport_commands[&ViewportId::ROOT];
	assert!(cmds.contains(&Cmd::CancelClose) && cmds.contains(&Cmd::Visible(true)));
	assert!(state.system_close() && !state.hidden);
	state.cancel_quit();
	assert!(!state.system_close());
	let _ = ctx.run_logic(&close, |ctx| state.logic(ctx, true, true, Some(true)));
	let _ = ctx.run_logic(&egui::RawInput::default(), |ctx| {
		state.logic(ctx, true, true, Some(false))
	});
	assert!(state.hidden);
	let output = ctx.run_logic(&egui::RawInput::default(), |ctx| {
		state.logic(ctx, false, true, Some(true))
	});
	assert!(!state.hidden);
	assert!(output.viewport_commands[&ViewportId::ROOT].contains(&Cmd::Visible(true)));
	let output = ctx.run_logic(&close, |ctx| {
		state.quit(ctx);
		state.logic(ctx, true, true, Some(true));
	});
	assert!(!output.viewport_commands[&ViewportId::ROOT].contains(&Cmd::Close));
	let mut output = ctx.run_ui(egui::RawInput::default(), |ui| state.ui(ui.ctx()));
	output.textures_delta.clear();
	assert!(
		output.viewport_output[&ViewportId::ROOT]
			.commands
			.contains(&Cmd::Close)
	);
	let _ = ctx.run_logic(&close, |ctx| {
		state.logic(ctx, true, true, Some(true));
		assert!(ctx.input(|i| i.viewport().close_requested()));
	});
	state.cancel_quit();
	let _ = ctx.run_logic(&close, |ctx| state.logic(ctx, true, true, Some(true)));
	let _ = ctx.run_logic(&egui::RawInput::default(), |ctx| {
		state.logic(ctx, true, true, Some(false))
	});
	assert!(state.hidden);

	// A compositor that ignores the hide must not strand an unclosable window: once the
	// confirmation budget runs out the app closes, through a close request the desktop's
	// own exit checks still see and can cancel.
	state.show(&ctx);
	let _ = ctx.run_logic(&close, |ctx| state.logic(ctx, true, true, Some(true)));
	let mut closed = false;
	for _ in 0..10 {
		let output = ctx.run_logic(&egui::RawInput::default(), |ctx| {
			state.logic(ctx, true, true, Some(true))
		});
		if output
			.viewport_commands
			.get(&ViewportId::ROOT)
			.is_some_and(|cmds| cmds.contains(&Cmd::Close))
		{
			closed = true;
			break;
		}
	}
	assert!(
		closed,
		"an ignored hide closes instead of stranding the window"
	);
	assert!(
		!state.hidden,
		"a failed hide never claims the window is hidden"
	);

	// Wayland reports neither visibility nor minimization, so an unconfirmable hide keeps
	// the previous tray behavior and never closes the app.
	state.cancel_quit();
	state.show(&ctx);
	let _ = ctx.run_logic(&close, |ctx| state.logic(ctx, true, false, None));
	let mut wayland_closed = false;
	for _ in 0..10 {
		let output = ctx.run_logic(&egui::RawInput::default(), |ctx| {
			state.logic(ctx, true, false, None);
		});
		wayland_closed |= output
			.viewport_commands
			.get(&ViewportId::ROOT)
			.is_some_and(|cmds| cmds.contains(&Cmd::Close));
	}
	assert!(
		!wayland_closed,
		"an unobservable Wayland hide never closes the app"
	);
	assert!(
		!state.hidden,
		"an unobservable hide is not claimed as hidden"
	);
	println!(
		"PASS: close/hide, hide confirmation, ignored-hide fallback, unobservable Wayland hide, hidden-window session close, host loss, deferred Quit and cancelled Quit."
	);
}
