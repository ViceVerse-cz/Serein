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
	restore: Box<dyn Fn() + Send + Sync>,
}

impl Events {
	fn push(&self, event: Event) {
		if event == Event::Unavailable {
			self.availability.store(2, Ordering::Release);
		}
		// A hidden window may receive no frames, so the UI would never see this event.
		if event != Event::Minimize {
			(self.restore)();
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
	/// `restore` runs on the tray worker for events that need a visible window, before `wake`.
	pub fn new(
		wake: impl Fn() + Send + Sync + 'static,
		restore: impl Fn() + Send + Sync + 'static,
	) -> Result<Self, &'static str> {
		let runtime = tokio::runtime::Handle::try_current()
			.map_err(|_| "The tray requires the application runtime.")?;
		let events = Arc::new(Events {
			bits: AtomicU8::new(0),
			availability: AtomicU8::new(0),
			wake: Box::new(wake),
			restore: Box::new(restore),
		});
		let (stop, mut stopped) = oneshot::channel();
		let worker_events = events.clone();
		runtime.spawn(async move {
			let Ok(icon) = image::load_from_memory_with_format(
				include_bytes!("../../../../assets/brand/serein-tray.png"),
				image::ImageFormat::Png,
			) else {
				worker_events.push(Event::Unavailable);
				return;
			};
			let icon = icon.into_rgba8();
			let (width, height) = (icon.width() as i32, icon.height() as i32);
			let mut pixels = icon.into_raw();
			for pixel in pixels.as_chunks_mut::<4>().0 {
				// Monochrome mark with no plate: keep only the shape. White suits the dark
				// panels of GNOME and most others; hosts that recolor use `icon_name` instead.
				pixel[0] = 255;
				pixel[1] = 255;
				pixel[2] = 255;
				pixel.rotate_right(1); // StatusNotifier pixmaps use ARGB, not RGBA.
			}
			// Flatpak exports `{FLATPAK_ID}-symbolic` into the host theme. A native
			// package exports `serein-symbolic`. Anything else leaves the name empty
			// so the host uses the pixmap: Quickshell prefers IconName and substitutes
			// an opaque missing icon when the name is absent, and Omarchy then masks
			// every `*-symbolic` icon into a solid square.
			let flatpak_id = std::env::var("FLATPAK_ID").ok();
			let flatpak_id = flatpak_id.as_deref().filter(|id| !id.is_empty());
			let native_installed = flatpak_id.is_none() && native_symbolic_icon_installed();
			let icon_name = symbolic_icon_name(flatpak_id, native_installed);
			let theme_path = icon_name
				.as_deref()
				.and_then(write_symbolic_icon)
				.unwrap_or_default();
			let item = Item {
				events: worker_events.clone(),
				pixels,
				size: [width, height],
				icon_name: icon_name.unwrap_or_default(),
				theme_path,
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

	pub fn set_voice_state(&self, _state: super::VoiceState) {}

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

/// Symbolic icon name; GNOME's AppIndicator extension and Plasma recolor these to match the
/// panel, so the tray mark follows the shell's own foreground colour like a native icon.
const ICON_NAME: &str = "serein-symbolic";

/// Name advertised on the StatusNotifier item.
///
/// A Flatpak id is used only when it is a safe icon name. An invalid id does not fall
/// back to the native name: the sandbox does not install that icon on the host.
fn symbolic_icon_name(flatpak_id: Option<&str>, native_installed: bool) -> Option<String> {
	if let Some(id) = flatpak_id {
		return safe_icon_id(id).map(|id| format!("{id}-symbolic"));
	}
	native_installed.then(|| ICON_NAME.to_owned())
}

fn safe_icon_id(id: &str) -> Option<&str> {
	if id.is_empty() || id.len() > 120 || id.contains("..") {
		return None;
	}
	let mut bytes = id.bytes();
	let first = bytes.next()?;
	if !first.is_ascii_alphanumeric() {
		return None;
	}
	if bytes.all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-')) {
		Some(id)
	} else {
		None
	}
}

fn native_symbolic_icon_installed() -> bool {
	fn data_dirs() -> Vec<std::path::PathBuf> {
		let mut dirs = Vec::new();
		if let Some(path) = std::env::var_os("XDG_DATA_HOME")
			.map(std::path::PathBuf::from)
			.filter(|path| path.is_absolute())
		{
			dirs.push(path);
		} else if let Some(home) = std::env::var_os("HOME")
			.map(std::path::PathBuf::from)
			.filter(|path| path.is_absolute())
		{
			dirs.push(home.join(".local/share"));
		}
		// XDG treats an empty XDG_DATA_DIRS like an unset one.
		match std::env::var_os("XDG_DATA_DIRS").filter(|list| !list.is_empty()) {
			Some(list) => {
				dirs.extend(std::env::split_paths(&list).filter(|path| path.is_absolute()))
			}
			None => {
				dirs.push(std::path::PathBuf::from("/usr/local/share"));
				dirs.push(std::path::PathBuf::from("/usr/share"));
			}
		}
		dirs
	}

	data_dirs().into_iter().any(|dir| {
		let Ok(sizes) = std::fs::read_dir(dir.join("icons/hicolor")) else {
			return false;
		};
		sizes.take(64).flatten().any(|size| {
			let apps = size.path().join("apps");
			["svg", "png"]
				.into_iter()
				.any(|ext| apps.join(format!("{ICON_NAME}.{ext}")).is_file())
		})
	})
}

/// Write the mark as a symbolic SVG into a private runtime directory and return that
/// directory, to be advertised as the item's icon theme path.
fn write_symbolic_icon(icon_name: &str) -> Option<String> {
	use std::os::unix::fs::DirBuilderExt as _;
	let stem = icon_name.strip_suffix("-symbolic")?;
	safe_icon_id(stem)?;
	let base = std::env::var_os("XDG_RUNTIME_DIR")
		.map(std::path::PathBuf::from)
		.filter(|path| path.is_absolute())?;
	let directory = base.join("serein-tray-icons");
	std::fs::DirBuilder::new()
		.recursive(true)
		.mode(0o700)
		.create(&directory)
		.ok()?;
	// Symbolic icons are recolored from this placeholder foreground.
	let svg = include_str!("../../../../assets/brand/serein-mark.svg")
		.replace("fill=\"#fff\"", "fill=\"#bebebe\"");
	std::fs::write(directory.join(format!("{icon_name}.svg")), svg).ok()?;
	directory.to_str().map(str::to_owned)
}

struct Item {
	events: Arc<Events>,
	pixels: Vec<u8>,
	size: [i32; 2],
	icon_name: String,
	theme_path: String,
}

impl ksni::Tray for Item {
	fn id(&self) -> String {
		"serein".into()
	}
	fn title(&self) -> String {
		"Serein".into()
	}
	fn icon_name(&self) -> String {
		// Empty when no host theme icon exists. The pixmap then carries the mark.
		self.icon_name.clone()
	}
	fn icon_theme_path(&self) -> String {
		self.theme_path.clone()
	}
	fn icon_pixmap(&self) -> Vec<ksni::Icon> {
		vec![ksni::Icon {
			width: self.size[0],
			height: self.size[1],
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

#[cfg(test)]
mod tests {
	use super::symbolic_icon_name;

	#[test]
	fn flatpak_id_selects_the_exported_symbolic_icon() {
		assert_eq!(
			symbolic_icon_name(Some("cz.viceverse.serein"), false).as_deref(),
			Some("cz.viceverse.serein-symbolic")
		);
		// An installed native icon must not replace a valid Flatpak id.
		assert_eq!(
			symbolic_icon_name(Some("cz.viceverse.serein"), true).as_deref(),
			Some("cz.viceverse.serein-symbolic")
		);
	}

	#[test]
	fn unsafe_flatpak_id_does_not_fall_back_to_the_native_name() {
		for id in [
			"",
			".hidden",
			"../outside",
			"a/b",
			"a..b",
			"-flag",
			"has space",
			"a\nb",
		] {
			assert_eq!(symbolic_icon_name(Some(id), true), None, "{id}");
		}
		let long = "a".repeat(121);
		assert_eq!(symbolic_icon_name(Some(&long), true), None);
	}

	#[test]
	fn unpackaged_run_uses_the_pixmap_unless_the_native_icon_is_installed() {
		assert_eq!(symbolic_icon_name(None, false), None);
		assert_eq!(
			symbolic_icon_name(None, true).as_deref(),
			Some("serein-symbolic")
		);
	}
}
