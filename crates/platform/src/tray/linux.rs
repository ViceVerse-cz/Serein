use super::Event;
use ksni::{TrayMethods, menu::StandardItem};
use std::sync::{
	Arc,
	atomic::{AtomicU8, Ordering},
};
use std::time::Duration;
use tokio::sync::oneshot;

struct Events {
	bits: AtomicU8,
	// Pending -> ready, or permanently unavailable until this registration is replaced.
	availability: AtomicU8,
	wake: Box<dyn Fn() + Send + Sync>,
}

impl Events {
	fn push(&self, event: Event) {
		if event == Event::Unavailable {
			self.availability.store(2, Ordering::Release);
		}
		if self.bits.fetch_or(event as u8, Ordering::Relaxed) & event as u8 == 0 {
			(self.wake)();
		}
	}
}

/// One cancellable registration on the application's runtime, never a UI-thread D-Bus call.
pub struct Tray {
	events: Arc<Events>,
	_stop: oneshot::Sender<()>,
}

impl Tray {
	pub fn new(wake: impl Fn() + Send + Sync + 'static) -> Result<Self, &'static str> {
		let runtime = tokio::runtime::Handle::try_current()
			.map_err(|_| "The tray requires the application runtime.")?;
		let events = Arc::new(Events {
			bits: AtomicU8::new(0),
			availability: AtomicU8::new(0),
			wake: Box::new(wake),
		});
		let (stop, mut stopped) = oneshot::channel();
		let worker_events = events.clone();
		runtime.spawn(async move {
			let Ok(icon) = image::load_from_memory_with_format(
				include_bytes!("../../../../packaging/linux/hicolor/32x32/apps/serein.png"),
				image::ImageFormat::Png,
			) else {
				worker_events.push(Event::Unavailable);
				return;
			};
			let mut pixels = icon.into_rgba8().into_raw();
			for pixel in pixels.as_chunks_mut::<4>().0 {
				pixel.rotate_right(1); // StatusNotifier pixmaps use ARGB, not RGBA.
			}
			let item = Item {
				events: worker_events.clone(),
				pixels,
			};
			let registration = item.disable_dbus_name(true).spawn();
			let result = tokio::select! {
				_ = &mut stopped => return,
				result = tokio::time::timeout(Duration::from_secs(3), registration) => result,
			};
			let Ok(Ok(handle)) = result else {
				worker_events.push(Event::Unavailable);
				return;
			};
			// ksni subscribes to watcher changes after registration. Recheck after
			// subscribing so a host lost during registration cannot leave a phantom tray.
			let host_present = tokio::select! {
				_ = &mut stopped => false,
				result = tokio::time::timeout(Duration::from_secs(3), async {
					let bus = zbus::Connection::session().await?;
					let dbus = zbus::fdo::DBusProxy::new(&bus).await?;
					dbus.name_has_owner("org.kde.StatusNotifierWatcher".try_into().unwrap()).await
				}) => matches!(result, Ok(Ok(true))),
			};
			if !host_present || handle.is_closed() {
				worker_events.push(Event::Unavailable);
				let _ = tokio::time::timeout(Duration::from_secs(2), handle.shutdown()).await;
				return;
			}
			if worker_events
				.availability
				.compare_exchange(0, 1, Ordering::AcqRel, Ordering::Acquire)
				.is_ok()
			{
				(worker_events.wake)();
			}
			let _ = stopped.await;
			let _ = tokio::time::timeout(Duration::from_secs(2), handle.shutdown()).await;
		});
		Ok(Self {
			events,
			_stop: stop,
		})
	}
	pub fn is_available(&self) -> bool {
		self.events.availability.load(Ordering::Acquire) == 1
	}

	pub fn take_event(&self) -> Option<Event> {
		[
			Event::Quit,
			Event::Minimize,
			Event::Unavailable,
			Event::Show,
		]
		.into_iter()
		.find(|event| {
			self.events
				.bits
				.fetch_and(!(*event as u8), Ordering::Relaxed)
				& *event as u8
				!= 0
		})
	}
}

struct Item {
	events: Arc<Events>,
	pixels: Vec<u8>,
}

impl ksni::Tray for Item {
	fn id(&self) -> String {
		"serein".into()
	}
	fn title(&self) -> String {
		"Serein".into()
	}
	fn icon_pixmap(&self) -> Vec<ksni::Icon> {
		vec![ksni::Icon {
			width: 32,
			height: 32,
			data: self.pixels.clone(),
		}]
	}
	fn activate(&mut self, _x: i32, _y: i32) {
		self.events.push(Event::Show);
	}
	fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
		vec![
			StandardItem {
				label: "Show Serein".into(),
				activate: Box::new(|item: &mut Self| item.events.push(Event::Show)),
				..Default::default()
			}
			.into(),
			StandardItem {
				label: "Minimize Serein".into(),
				activate: Box::new(|item: &mut Self| item.events.push(Event::Minimize)),
				..Default::default()
			}
			.into(),
			StandardItem {
				label: "Quit".into(),
				activate: Box::new(|item: &mut Self| item.events.push(Event::Quit)),
				..Default::default()
			}
			.into(),
		]
	}
	fn watcher_offline(&self, _reason: ksni::OfflineReason) -> bool {
		self.events.push(Event::Unavailable);
		false
	}
}
