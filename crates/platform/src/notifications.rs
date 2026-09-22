//! Device-preference-controlled OS alerts. Message previews are bounded before reaching this worker.
use std::sync::{
	Arc,
	atomic::{AtomicU64, Ordering},
	mpsc::{self, Receiver, SyncSender},
};

// Windows delivers activation through its callback; other platforms retain one cancellable future.
#[cfg(target_os = "windows")]
type NotificationHandle = ();
#[cfg(not(target_os = "windows"))]
struct NotificationHandle {
	response: Option<std::pin::Pin<Box<dyn std::future::Future<Output = ()>>>>,
	close: Box<dyn FnOnce()>,
}

#[derive(Clone)]
struct Activation {
	send: SyncSender<(u64, model::Id)>,
	generation: u64,
	channel: Option<model::Id>,
	wake: Arc<dyn Fn() + Send + Sync>,
}
impl Activation {
	fn clicked(&self) {
		if let Some(channel) = self.channel
			&& self.send.try_send((self.generation, channel)).is_ok()
		{
			(self.wake)();
		}
	}
}

// Eight bounded commands; overflow drops an alert, never message state.
const QUEUE_ITEMS: usize = 8;
const GENERIC_BODY: &str = "You have a new message.";
const TITLE_BYTES: usize = 256;
const BODY_BYTES: usize = 512;
const IMAGE_PATH_BYTES: usize = 512;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Status {
	Disabled,
	Enabling,
	Ready,
	Denied,
	Unavailable,
	QueueFull,
}

impl Status {
	pub fn label(self) -> &'static str {
		match self {
			Self::Disabled => "System notifications are off in Serein settings.",
			Self::Enabling => "Checking system notification permission…",
			Self::Ready => {
				"Serein can send system notifications; message alerts show sender and preview."
			}
			Self::Denied => "System notifications are disabled in your OS settings.",
			Self::QueueFull => {
				"Notification queue full; an alert was skipped. Unread indicators are retained."
			}
			Self::Unavailable => {
				#[cfg(target_os = "macos")]
				{
					"System notifications unavailable. Run the packaged Serein.app and check System Settings > Notifications."
				}
				#[cfg(target_os = "windows")]
				{
					"System notifications unavailable. Register the packaged Start Menu shortcut with install-notifications.ps1 and check Windows notification settings."
				}
				#[cfg(not(any(target_os = "macos", target_os = "windows")))]
				{
					"System notifications unavailable. Check your desktop notification service and settings."
				}
			}
		}
	}
}

struct Alert {
	title: Box<str>,
	body: Box<str>,
	image_path: Option<Box<str>>,
}

struct Command {
	generation: u64,
	channel: Option<model::Id>,
	alert: Option<Alert>,
}

/// Created without OS calls or a thread. The worker starts only when the device setting is enabled.
pub struct Notifications {
	activated: Receiver<(u64, model::Id)>,
	activation_send: SyncSender<(u64, model::Id)>,
	send: Option<SyncSender<Command>>,
	generation: Arc<AtomicU64>,
	status: Arc<AtomicU64>,
	wake: Arc<dyn Fn() + Send + Sync>,
}

impl Notifications {
	pub fn new(wake: impl Fn() + Send + Sync + 'static) -> Self {
		let (activation_send, activated) = mpsc::sync_channel(QUEUE_ITEMS);
		Self {
			activated,
			activation_send,
			send: None,
			generation: Arc::new(AtomicU64::new(0)),
			status: Arc::new(AtomicU64::new(0)),
			wake: Arc::new(wake),
		}
	}

	/// Enable for this session only. macOS may ask for OS permission; never blocks rendering.
	pub fn set_enabled(&mut self, enabled: bool) {
		if (self.generation.load(Ordering::Acquire) & 1 != 0) == enabled {
			return;
		}
		let generation = self.generation.fetch_add(1, Ordering::AcqRel) + 1;
		self.status.store(
			encoded(
				generation,
				if enabled {
					Status::Enabling
				} else {
					Status::Disabled
				},
			),
			Ordering::Release,
		);
		if enabled && self.send.is_none() {
			let (send, receive) = mpsc::sync_channel(QUEUE_ITEMS);
			let current = Arc::clone(&self.generation);
			let status = Arc::clone(&self.status);
			let wake = Arc::clone(&self.wake);
			let activated = self.activation_send.clone();
			if std::thread::Builder::new()
				.name("serein-notifications".into())
				.spawn(move || worker(receive, current, status, wake, activated))
				.is_err()
			{
				self.status
					.store(encoded(generation, Status::Unavailable), Ordering::Release);
				return;
			}
			self.send = Some(send);
		}
		// If full, the next queued command also observes the changed generation and clears.
		if let Some(send) = &self.send {
			let _ = send.try_send(Command {
				generation,
				channel: None,
				alert: None,
			});
		}
	}

	/// Cancel queued alerts and close the outstanding alert where supported. OS history may remain.
	pub fn clear(&mut self) {
		self.set_enabled(false);
	}

	/// Invalidate pending/outstanding alerts after read, mute, DND or connection changes.
	/// Retains the session opt-in and does not request OS authorization again.
	pub fn dismiss(&mut self) {
		let result = match self.status() {
			Status::QueueFull => Status::Ready,
			result => result,
		};
		let generation = self.generation.fetch_add(2, Ordering::AcqRel) + 2;
		self.status
			.store(encoded(generation, result), Ordering::Release);
		if let Some(send) = &self.send {
			let _ = send.try_send(Command {
				generation,
				channel: None,
				alert: None,
			});
		}
	}

	/// Queue a privacy-preserving generic alert. False means disabled, unavailable or overloaded.
	pub fn notify(&self) -> bool {
		self.enqueue(Alert {
			title: "Serein".into(),
			body: GENERIC_BODY.into(),
			image_path: None,
		})
	}
	pub fn notify_message(&self, title: String, body: String, image_path: Option<String>) -> bool {
		self.message(None, title, body, image_path)
	}
	pub fn notify_channel(
		&self,
		channel: model::Id,
		title: String,
		body: String,
		image_path: Option<String>,
	) -> bool {
		self.message(Some(channel), title, body, image_path)
	}
	pub fn take_activation(&self) -> Option<model::Id> {
		let current = self.generation.load(Ordering::Acquire);
		self.activated.try_iter().find_map(|(generation, channel)| {
			(current & 1 != 0 && current == generation).then_some(channel)
		})
	}
	fn message(
		&self,
		channel: Option<model::Id>,
		title: String,
		body: String,
		image_path: Option<String>,
	) -> bool {
		if title.len() > TITLE_BYTES
			|| body.len() > BODY_BYTES
			|| image_path
				.as_ref()
				.is_some_and(|path| path.len() > IMAGE_PATH_BYTES)
		{
			return false;
		}
		self.enqueue_channel(
			Alert {
				title: title.into_boxed_str(),
				body: body.into_boxed_str(),
				image_path: image_path.map(String::into_boxed_str),
			},
			channel,
		)
	}
	fn enqueue(&self, alert: Alert) -> bool {
		self.enqueue_channel(alert, None)
	}
	fn enqueue_channel(&self, alert: Alert, channel: Option<model::Id>) -> bool {
		if !matches!(self.status(), Status::Ready | Status::QueueFull) {
			return false;
		}
		let generation = self.generation.load(Ordering::Acquire);
		let Some(send) = &self.send else { return false };
		match send.try_send(Command {
			generation,
			channel,
			alert: Some(alert),
		}) {
			Ok(()) => true,
			Err(error) => {
				let status = match error {
					mpsc::TrySendError::Full(_) => Status::QueueFull,
					mpsc::TrySendError::Disconnected(_) => Status::Unavailable,
				};
				self.status
					.store(encoded(generation, status), Ordering::Release);
				false
			}
		}
	}

	pub fn status(&self) -> Status {
		let generation = self.generation.load(Ordering::Acquire);
		let value = self.status.load(Ordering::Acquire);
		if value >> 8 != generation {
			return if generation & 1 == 0 {
				Status::Disabled
			} else {
				Status::Enabling
			};
		}
		match value & 255 {
			0 => Status::Disabled,
			1 => Status::Enabling,
			2 => Status::Ready,
			3 => Status::Denied,
			5 => Status::QueueFull,
			_ => Status::Unavailable,
		}
	}
}

impl Drop for Notifications {
	fn drop(&mut self) {
		self.clear();
		// Dropping the sole sender wakes the worker; never join an OS call on the UI thread.
	}
}

fn encoded(generation: u64, status: Status) -> u64 {
	(generation << 8) | status as u64
}

fn worker(
	receive: Receiver<Command>,
	current: Arc<AtomicU64>,
	status: Arc<AtomicU64>,
	wake: Arc<dyn Fn() + Send + Sync>,
	activated: SyncSender<(u64, model::Id)>,
) {
	let mut generation = 0;
	let mut outcome = Status::Disabled;
	let mut outstanding: Option<NotificationHandle> = None;
	loop {
		#[cfg(not(target_os = "windows"))]
		if let Some(handle) = &mut outstanding
			&& let Some(response) = &mut handle.response
			&& response
				.as_mut()
				.poll(&mut std::task::Context::from_waker(std::task::Waker::noop()))
				.is_ready()
		{
			handle.response = None;
		}
		#[cfg(not(target_os = "windows"))]
		let waiting = outstanding
			.as_ref()
			.is_some_and(|handle| handle.response.is_some());
		#[cfg(target_os = "windows")]
		let waiting = false;
		let command = if waiting {
			match receive.recv_timeout(std::time::Duration::from_millis(100)) {
				Ok(command) => command,
				Err(mpsc::RecvTimeoutError::Timeout) => continue,
				Err(mpsc::RecvTimeoutError::Disconnected) => break,
			}
		} else {
			let Ok(command) = receive.recv() else { break };
			command
		};
		let active = current.load(Ordering::Acquire);
		if active != generation {
			close(&mut outstanding);
			let was_enabled = generation & 1 != 0;
			generation = active;
			outcome = if generation & 1 == 0 {
				Status::Disabled
			} else if !was_enabled
				|| status.load(Ordering::Acquire) == encoded(generation, Status::Enabling)
			{
				authorize()
			} else {
				outcome
			};
			publish(&status, generation, outcome);
			wake();
		}
		if outcome != Status::Ready
			|| command.alert.is_none()
			|| command.generation != generation
			|| current.load(Ordering::Acquire) != generation
		{
			continue;
		}
		// Keep at most one notification/response handle, including in OS history where supported.
		close(&mut outstanding);
		let activation = Activation {
			send: activated.clone(),
			generation,
			channel: command.channel,
			wake: Arc::clone(&wake),
		};
		outcome = match show(command.alert.as_ref().expect("checked above"), activation) {
			Ok(handle) => {
				outstanding = Some(handle);
				Status::Ready
			}
			Err(()) => Status::Unavailable,
		};
		if current.load(Ordering::Acquire) != generation {
			close(&mut outstanding);
		}
		publish(&status, generation, outcome);
		wake();
	}
	close(&mut outstanding);
}

fn publish(status: &AtomicU64, generation: u64, outcome: Status) {
	// An OS result must not overwrite the next session/dismissal's state.
	let _ = status.fetch_update(Ordering::AcqRel, Ordering::Acquire, |current| {
		(current >> 8 == generation).then_some(encoded(generation, outcome))
	});
}

fn authorize() -> Status {
	#[cfg(target_os = "macos")]
	{
		match notify_rust::request_auth_blocking() {
			Ok(true) => Status::Ready,
			Ok(false) => Status::Denied,
			Err(_) => Status::Unavailable,
		}
	}
	#[cfg(target_os = "windows")]
	{
		use windows::{UI::Notifications::ToastNotificationManager, core::HSTRING};
		let notifier = ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(
			"cz.viceverse.serein",
		));
		match notifier {
			Ok(notifier) => windows_setting_status(notifier.Setting(), windows_shortcut_exists()),
			Err(_) => Status::Unavailable,
		}
	}
	#[cfg(not(any(target_os = "macos", target_os = "windows")))]
	{
		Status::Ready
	}
}

#[cfg(target_os = "windows")]
fn windows_setting_status(
	setting: windows::core::Result<windows::UI::Notifications::NotificationSetting>,
	has_shortcut: bool,
) -> Status {
	use windows::{UI::Notifications::NotificationSetting, core::HRESULT};
	match setting {
		Ok(NotificationSetting::Enabled) => Status::Ready,
		Ok(_) => Status::Denied,
		// Windows may not have a settings entry for an unpackaged desktop app yet.
		// Its notifier can still submit a toast; show() reports delivery errors.
		Err(error) if has_shortcut && error.code() == HRESULT(0x80070490_u32 as i32) => {
			Status::Ready
		}
		Err(_) => Status::Unavailable,
	}
}

#[cfg(target_os = "windows")]
fn windows_shortcut_exists() -> bool {
	std::env::var_os("APPDATA").is_some_and(|root| {
		std::path::PathBuf::from(root)
			.join("Microsoft/Windows/Start Menu/Programs/Serein.lnk")
			.is_file()
	})
}

fn show(alert: &Alert, activation: Activation) -> Result<NotificationHandle, ()> {
	#[cfg(target_os = "macos")]
	{
		// The blocking wrapper mistakes a busy AppKit run loop for a stopped one.
		// Await the OS completion on this worker; never block the native UI thread.
		let native = mac_usernotifications::Notification::from(&notification(alert));
		let handle = futures_lite::future::block_on(native.send()).map_err(|_| ())?;
		let id = handle.notification_id().to_owned();
		Ok(NotificationHandle {
			response: Some(Box::pin(async move {
				if handle
					.response()
					.await
					.is_ok_and(|response| response.is_default_action())
				{
					activation.clicked();
				}
			})),
			close: Box::new(move || {
				futures_lite::future::block_on(mac_usernotifications::close_delivered(&id))
			}),
		})
	}
	#[cfg(not(any(target_os = "macos", target_os = "windows")))]
	{
		let handle = Arc::new(notification(alert).show().map_err(|_| ())?);
		let response_handle = Arc::clone(&handle);
		Ok(NotificationHandle {
			response: Some(Box::pin(async move {
				response_handle
					.wait_for_action_async(|response| {
						if response.is_default_action() {
							activation.clicked();
						}
					})
					.await;
			})),
			close: Box::new(move || {
				if let Ok(handle) = Arc::try_unwrap(handle) {
					handle.close();
				}
			}),
		})
	}
	#[cfg(target_os = "windows")]
	{
		use tauri_winrt_notification::{IconCrop, Toast};
		let mut toast = Toast::new("cz.viceverse.serein")
			.title(&alert.title)
			.text1(&alert.body)
			.on_activated(move |_| {
				activation.clicked();
				Ok(())
			})
			.sound(None);
		if let Some(path) = &alert.image_path
			&& std::path::Path::new(path.as_ref()).is_file()
		{
			toast = toast.icon(
				std::path::Path::new(path.as_ref()),
				IconCrop::Circular,
				&alert.title,
			);
		}
		toast.show().map_err(|_| ())
	}
}

#[cfg(any(not(target_os = "windows"), test))]
fn notification(alert: &Alert) -> notify_rust::Notification {
	let mut notification = notify_rust::Notification::new();
	notification
		.appname("Serein")
		.summary(&alert.title)
		.body(&alert.body)
		.timeout(5_000);
	#[cfg(not(any(target_os = "macos", target_os = "windows")))]
	notification.action("default", "Open channel");
	if let Some(path) = &alert.image_path
		&& std::path::Path::new(path.as_ref()).is_file()
	{
		notification.image_path(path);
	}
	// Sound is played independently by the bounded local audio worker.
	#[cfg(target_os = "linux")]
	notification.hint(notify_rust::Hint::SuppressSound(true));
	notification
}

fn close(outstanding: &mut Option<NotificationHandle>) {
	#[cfg(not(target_os = "windows"))]
	if let Some(handle) = outstanding.take() {
		drop(handle.response);
		(handle.close)();
	}
	#[cfg(target_os = "windows")]
	if outstanding.take().is_some() {
		use windows::{UI::Notifications::ToastNotificationManager, core::HSTRING};
		let _ = ToastNotificationManager::History()
			.and_then(|history| history.ClearWithId(&HSTRING::from("cz.viceverse.serein")));
	}
}

/// Synthetic handoff check: never sends an OS notification or starts a worker.
#[cfg(debug_assertions)]
pub fn debug_activation_check(channel: model::Id) -> model::Id {
	let mut notifications = Notifications::new(|| {});
	notifications.generation.store(1, Ordering::Release);
	let click = Activation {
		send: notifications.activation_send.clone(),
		generation: 1,
		channel: Some(channel),
		wake: Arc::clone(&notifications.wake),
	};
	click.clicked();
	let selected = notifications
		.take_activation()
		.expect("click retains its channel");
	assert_eq!(selected, channel);
	click.clicked();
	notifications.dismiss();
	assert_eq!(notifications.take_activation(), None);
	click.clicked();
	notifications.clear();
	assert_eq!(notifications.take_activation(), None);
	selected
}

#[cfg(test)]
mod tests {
	use super::*;
	#[cfg(target_os = "windows")]
	#[test]
	fn missing_windows_settings_entry_allows_delivery_but_denials_do_not() {
		use windows::{
			UI::Notifications::NotificationSetting,
			core::{Error, HRESULT},
		};
		assert_eq!(
			windows_setting_status(
				Err(Error::from_hresult(HRESULT(0x80070490_u32 as i32))),
				true
			),
			Status::Ready
		);
		assert_eq!(
			windows_setting_status(
				Err(Error::from_hresult(HRESULT(0x80070490_u32 as i32))),
				false
			),
			Status::Unavailable
		);
		assert_eq!(
			windows_setting_status(Ok(NotificationSetting::DisabledForUser), true),
			Status::Denied
		);
		assert_eq!(
			windows_setting_status(
				Err(Error::from_hresult(HRESULT(0x80004005_u32 as i32))),
				true
			),
			Status::Unavailable
		);
	}

	#[test]
	fn disabled_is_lazy_and_fixed_queue_is_bounded_and_invalidated() {
		let alert = notification(&Alert {
			title: "Serein".into(),
			body: GENERIC_BODY.into(),
			image_path: None,
		});
		assert_eq!(alert.summary, "Serein");
		assert_eq!(alert.body, "You have a new message.");
		let mut notifications = Notifications::new(|| {});
		assert_eq!(notifications.status(), Status::Disabled);
		assert!(notifications.send.is_none());
		assert!(!notifications.notify());
		// Substitute only the queue; tests never authorize or contact an OS notification service.
		let (send, receive) = mpsc::sync_channel(QUEUE_ITEMS);
		notifications.send = Some(send);
		notifications.generation.store(1, Ordering::Release);
		notifications
			.status
			.store(encoded(1, Status::Ready), Ordering::Release);
		for _ in 0..QUEUE_ITEMS {
			assert!(notifications.notify());
		}
		assert!(!notifications.notify());
		assert_eq!(notifications.status(), Status::QueueFull);
		notifications.dismiss();
		assert_eq!(notifications.status(), Status::Ready);
		for command in receive.try_iter().filter(|command| command.alert.is_some()) {
			assert_ne!(
				command.generation,
				notifications.generation.load(Ordering::Acquire)
			);
		}
		assert!(notifications.notify());
		notifications.clear();
		assert_eq!(notifications.status(), Status::Disabled);
		assert!(!notifications.notify());
		for command in receive.try_iter().filter(|command| command.alert.is_some()) {
			assert_ne!(
				command.generation,
				notifications.generation.load(Ordering::Acquire)
			);
		}
		assert!(
			(std::mem::size_of::<Command>() + TITLE_BYTES + BODY_BYTES + IMAGE_PATH_BYTES)
				* QUEUE_ITEMS
				<= 16 * 1024
		);
		let active_status = notifications.status.load(Ordering::Acquire);
		publish(&notifications.status, 1, Status::Unavailable);
		assert_eq!(notifications.status.load(Ordering::Acquire), active_status);
		assert_eq!(notifications.status(), Status::Disabled);
	}
	#[test]
	fn message_alert_payload_is_bounded_and_retains_preview() {
		let mut notifications = Notifications::new(|| {});
		let (send, receive) = mpsc::sync_channel(QUEUE_ITEMS);
		notifications.send = Some(send);
		notifications.generation.store(1, Ordering::Release);
		notifications
			.status
			.store(encoded(1, Status::Ready), Ordering::Release);
		assert!(notifications.notify_message("A sender".into(), "Hello there".into(), None));
		let alert = receive.try_recv().unwrap().alert.unwrap();
		assert_eq!((&*alert.title, &*alert.body), ("A sender", "Hello there"));
		assert!(!notifications.notify_message("x".repeat(TITLE_BYTES + 1), "Message".into(), None));
		assert!(receive.try_recv().is_err());
	}
}
