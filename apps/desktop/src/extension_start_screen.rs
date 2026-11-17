//! The device-level start screen a granted plugin last set. It shows before any account loads,
//! so the accepted value is kept in one bounded file beside the other extension state and read
//! at launch without running plugin code; its frames are decoded once, when it is loaded or
//! accepted. When several plugins set one, the lowest plugin id wins, like custom Rich Presence
//! and interface replacement. Disabling, removing or failing the owning plugin, revoking its
//! grant, ending its account session or its explicit `null` clears it.

use crate::extensions::{account_key, read_bounded, write_device_file};
use eframe::egui;
use extensions::StartScreen;
use serde::{Deserialize, Serialize};
use std::{
	path::{Path, PathBuf},
	sync::{
		Arc,
		mpsc::{self, TryRecvError},
	},
	time::Instant,
};

/// Write attempts behind one unsaved value, including the first.
const MAX_SAVE_ATTEMPTS: u32 = 3;
/// Wait before every retry after the first; later polls back off instead of hot-looping.
const RETRY_DELAY: std::time::Duration = std::time::Duration::from_secs(1);
/// The failure reported when a save cannot start or its writer dies.
const SAVE_ERROR: &str = "Start screen could not be saved";
/// The failure a screen over the file budget always reports; retrying cannot fix it.
const BUDGET_ERROR: &str = "Start screen exceeds its device storage budget";

/// File beside the extension directories.
const FILE: &str = "start-screen.json";
/// Largest saved file: the base64 of the most packed frame bytes a valid screen holds (about
/// 214 KiB), plus 16 KiB for its JSON structure, text, palettes, window size and owner.
pub const MAX_FILE_BYTES: usize = extensions::MAX_START_FRAME_BYTES.div_ceil(3) * 4 + 16 * 1024;

/// The saved screen and its owner. The account is the SHA-256 key that also names the
/// account's extension directory, so the file holds no account identifier.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Saved {
	account: String,
	plugin: String,
	screen: StartScreen,
}

#[derive(Serialize)]
struct SavedRef<'a> {
	account: &'a str,
	plugin: &'a str,
	screen: &'a StartScreen,
}

#[derive(Clone)]
struct Owner {
	account: String,
	plugin: String,
	screen: Arc<ui::DecodedStartScreen>,
}

impl Owner {
	fn same(this: Option<&Self>, other: Option<&Self>) -> bool {
		match (this, other) {
			(None, None) => true,
			(Some(this), Some(other)) => {
				this.account == other.account
					&& this.plugin == other.plugin
					&& Arc::ptr_eq(&this.screen, &other.screen)
			}
			_ => false,
		}
	}
}
/// The start screen kept on this device. Persistence runs on a short-lived thread: at most
/// one write is in flight and only the newest unsaved value waits behind it. A failed
/// write keeps its value and is retried a bounded number of times, later retries after a
/// delay; a screen over the file budget is never retried, because no attempt can fit it.
#[derive(Default)]
pub struct DeviceStartScreen {
	path: Option<PathBuf>,
	context: Option<egui::Context>,
	owner: Option<Owner>,
	/// A change not yet handed to a writer; `Some(None)` removes the file.
	unsaved: Option<Option<Owner>>,
	writing: Option<mpsc::Receiver<Result<(), String>>>,
	/// The value handed to the in-flight writer, kept for its bounded retry.
	in_flight: Option<Option<Owner>>,
	/// Failed attempts behind the current unsaved value.
	attempts: u32,
	/// When the next retry may start; `None` retries on the next poll.
	retry_at: Option<Instant>,
}

impl DeviceStartScreen {
	/// Reads the saved screen without running plugin code. A missing, oversized or invalid
	/// file leaves the default screen.
	pub fn load(root: &Path, context: &egui::Context) -> Self {
		let path = root.join(FILE);
		let owner = path
			.is_file()
			.then(|| read_bounded(&path, MAX_FILE_BYTES).ok())
			.flatten()
			.and_then(|bytes| serde_json::from_slice::<Saved>(&bytes).ok())
			.filter(|saved| {
				saved.account.len() == 64
					&& saved.account.bytes().all(|byte| byte.is_ascii_hexdigit())
					&& extensions::valid_id(&saved.plugin)
			})
			.and_then(|saved| {
				Some(Owner {
					screen: Arc::new(ui::DecodedStartScreen::new(saved.screen)?),
					account: saved.account,
					plugin: saved.plugin,
				})
			});
		Self {
			path: Some(path),
			context: Some(context.clone()),
			owner,
			..Self::default()
		}
	}

	pub fn current(&self) -> Option<Arc<ui::DecodedStartScreen>> {
		self.owner.as_ref().map(|owner| owner.screen.clone())
	}

	/// Reconciles the screen with one account's working, granted plugins and the start screen
	/// each last restated (`None`: nothing this session). The owner's own value replaces or
	/// clears it; an owner missing from `plugins` was disabled, removed, failed or lost its
	/// grant and clears it; the lowest plugin id with a screen takes over.
	pub fn reconcile<'a>(
		&mut self,
		account: &str,
		plugins: impl Iterator<Item = (&'a str, &'a Option<Option<StartScreen>>)> + Clone,
	) {
		let Ok(key) = account_key(account) else {
			return;
		};
		let mut next = self.owner.clone();
		if let Some(owner) = self.owner.as_ref().filter(|owner| owner.account == key) {
			match plugins.clone().find(|(id, _)| *id == owner.plugin) {
				None | Some((_, Some(None))) => next = None,
				Some((_, Some(Some(screen)))) if owner.screen.screen() != screen => {
					next = ui::DecodedStartScreen::new(screen.clone()).map(|screen| Owner {
						screen: Arc::new(screen),
						..owner.clone()
					});
				}
				Some(_) => {}
			}
		}
		let candidate = plugins
			.filter_map(|(id, screen)| Some((id, screen.as_ref()?.as_ref()?)))
			.min_by_key(|(id, _)| *id);
		// A current-account candidate always replaces an owner from another account; within
		// one account the lowest plugin id keeps the screen.
		if let Some((id, screen)) = candidate
			&& next
				.as_ref()
				.is_none_or(|owner| owner.account != key || id < owner.plugin.as_str())
			&& let Some(screen) = ui::DecodedStartScreen::new(screen.clone())
		{
			next = Some(Owner {
				account: key,
				plugin: id.to_owned(),
				screen: Arc::new(screen),
			});
		}
		if !Owner::same(next.as_ref(), self.owner.as_ref()) {
			self.owner = next;
			self.save();
		}
	}

	/// Clears a screen set by one of this account's plugins when its session ends, because
	/// ending it removes the account's plugins.
	pub fn release_account(&mut self, account: &str) {
		if let Ok(key) = account_key(account)
			&& self
				.owner
				.as_ref()
				.is_some_and(|owner| owner.account == key)
		{
			self.owner = None;
			self.save();
		}
	}

	/// Queues the current value; [`Self::poll`] hands it to a writer on this frame's tick.
	fn save(&mut self) {
		self.unsaved = Some(self.owner.clone());
		self.attempts = 0;
		self.retry_at = None;
	}

	fn flush(&mut self) -> Option<String> {
		if self.writing.is_some() {
			return None;
		}
		let path = self.path.clone()?;
		let owner = self.unsaved.take()?;
		let writing = owner.clone();
		let context = self.context.clone();
		let (sender, receiver) = mpsc::sync_channel(1);
		let spawned = std::thread::Builder::new()
			.name("serein-start-screen-save".into())
			.spawn(move || {
				let _ = sender.send(write(&path, writing.as_ref()));
				if let Some(context) = context {
					context.request_repaint();
				}
			});
		match spawned {
			Ok(_) => {
				self.writing = Some(receiver);
				self.in_flight = Some(owner);
				None
			}
			// The value stays queued; a later poll retries it behind the same bound.
			Err(_) => self.failed(owner, SAVE_ERROR),
		}
	}

	/// A failed write keeps its value for one more attempt unless none can succeed or a newer
	/// value already waits (that one supersedes it, with its own attempts); the first retry
	/// runs on the next frame and later ones wait out the backoff, each with a scheduled wake.
	/// Returns the failure to report either way.
	fn failed(&mut self, owner: Option<Owner>, error: &str) -> Option<String> {
		if self.unsaved.is_some() {
			return Some(error.to_owned());
		}
		if error == BUDGET_ERROR || self.attempts + 1 >= MAX_SAVE_ATTEMPTS {
			self.attempts = 0;
			self.retry_at = None;
		} else {
			self.attempts += 1;
			let delay = if self.attempts > 1 {
				RETRY_DELAY
			} else {
				std::time::Duration::ZERO
			};
			self.retry_at = (self.attempts > 1).then(|| Instant::now() + delay);
			self.unsaved = Some(owner);
			if let Some(context) = &self.context {
				context.request_repaint_after(delay);
			}
		}
		Some(error.to_owned())
	}

	/// Finishes a completed write and starts the newest waiting one — a failed write once
	/// its backoff elapses, while attempts remain; returns a failure. Called once per frame.
	pub fn poll(&mut self) -> Option<String> {
		let finished = match self.writing.as_ref().map(mpsc::Receiver::try_recv) {
			Some(Err(TryRecvError::Empty)) => return None,
			Some(Ok(result)) => result.err(),
			Some(Err(TryRecvError::Disconnected)) => Some(SAVE_ERROR.into()),
			None => None,
		};
		self.writing = None;
		if let Some(owner) = self.in_flight.take()
			&& let Some(error) = &finished
		{
			return self.failed(owner, error);
		}
		if let Some(at) = self.retry_at
			&& let Some(wait) = at.checked_duration_since(Instant::now())
		{
			if let Some(context) = &self.context {
				context.request_repaint_after(wait);
			}
			return None;
		}
		self.retry_at = None;
		self.flush().or(finished)
	}

	/// Whether a save is queued or in flight; the desktop's close gate waits for it.
	pub fn pending(&self) -> bool {
		self.writing.is_some() || self.unsaved.is_some()
	}

	/// Blocks until every queued write has finished.
	#[cfg(test)]
	pub fn settle(&mut self) {
		loop {
			assert_eq!(self.flush(), None);
			self.attempts = 0;
			self.retry_at = None;
			let Some(receiver) = self.writing.take() else {
				return;
			};
			assert_eq!(receiver.recv(), Ok(Ok(())));
			self.in_flight = None;
		}
	}
}

fn write(path: &Path, owner: Option<&Owner>) -> Result<(), String> {
	let Some(owner) = owner else {
		return write_device_file(path, None);
	};
	let bytes = serde_json::to_vec(&SavedRef {
		account: &owner.account,
		plugin: &owner.plugin,
		screen: owner.screen.screen(),
	})
	.map_err(|_| "Cannot encode the start screen")?;
	if bytes.len() > MAX_FILE_BYTES {
		return Err(BUDGET_ERROR.into());
	}
	write_device_file(path, Some(&bytes))
}

#[cfg(test)]
mod tests {
	use super::*;

	/// Checks that the largest valid screen, saved with its owner, fits the file cap: every
	/// frame byte the limit allows, the most frames and images, fully escaped text and the
	/// longest palette colors and plugin id.
	#[test]
	fn largest_valid_screen_fits_the_file_cap() {
		// 31×44 cells pack into 341 bytes: 480 frames hold 163,680 of the 163,840 allowed.
		let image = extensions::StartImage {
			width: 31,
			height: 44,
			palette: vec!["#00000000".into(); extensions::MAX_START_PALETTE],
			frames: vec![format!("{}=", "A".repeat(455)); extensions::MAX_START_FRAMES],
			fps: extensions::MAX_START_FPS,
			cell: extensions::MAX_START_CELL_PIXELS,
		};
		let screen = StartScreen {
			images: vec![image; extensions::MAX_START_IMAGES],
			title: Some("\"".repeat(extensions::MAX_START_TITLE_BYTES)),
			subtitle: Some("\\".repeat(extensions::MAX_START_SUBTITLE_BYTES)),
			window: Some(extensions::StartWindow {
				width: extensions::MAX_START_WINDOW_WIDTH,
				height: extensions::MAX_START_WINDOW_HEIGHT,
			}),
		};
		assert_eq!(screen.validate(), Ok(()));
		let bytes = serde_json::to_vec(&SavedRef {
			account: &"0".repeat(64),
			plugin: &"a".repeat(64),
			screen: &screen,
		})
		.unwrap();
		assert!(bytes.len() <= MAX_FILE_BYTES, "{} bytes", bytes.len());
	}

	/// A failed write must not bring back an older value over a newer one already queued
	/// (e.g. a logout's clear queued while a screen was being written).
	#[test]
	fn failed_write_keeps_a_newer_queued_value() {
		let mut device = DeviceStartScreen {
			unsaved: Some(None),
			..Default::default()
		};
		let screen = StartScreen {
			title: Some("Old".into()),
			..Default::default()
		};
		let old = Owner {
			account: "a".into(),
			plugin: "p".into(),
			screen: Arc::new(ui::DecodedStartScreen::new(screen).expect("valid screen")),
		};
		assert!(device.failed(Some(old), SAVE_ERROR).is_some());
		assert!(
			matches!(device.unsaved, Some(None)),
			"the queued clear survives"
		);
		assert_eq!(device.attempts, 0);
	}
}
