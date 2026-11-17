//! The device-level start screen a granted plugin last set. It shows before any account loads,
//! so the accepted value is kept in one bounded file beside the other extension state and read
//! at launch without running plugin code. When several plugins set one, the lowest plugin id
//! wins, like custom Rich Presence and interface replacement. Disabling, removing or failing
//! the owning plugin, revoking its grant, ending its account session or its explicit `null`
//! clears it.

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
};

/// File beside the extension directories.
const FILE: &str = "start-screen.json";
/// Largest saved file: one plugin result's screen, its owner and re-encoding headroom.
pub const MAX_FILE_BYTES: usize = extensions::MAX_IO_BYTES + 64 * 1024;

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
	screen: Arc<StartScreen>,
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
/// one write is in flight and only the newest unsaved value waits behind it.
#[derive(Default)]
pub struct DeviceStartScreen {
	path: Option<PathBuf>,
	context: Option<egui::Context>,
	owner: Option<Owner>,
	/// A change not yet handed to a writer; `Some(None)` removes the file.
	unsaved: Option<Option<Owner>>,
	writing: Option<mpsc::Receiver<Result<(), String>>>,
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
					&& saved.screen.validate().is_ok()
			})
			.map(|saved| Owner {
				account: saved.account,
				plugin: saved.plugin,
				screen: Arc::new(saved.screen),
			});
		Self {
			path: Some(path),
			context: Some(context.clone()),
			owner,
			..Self::default()
		}
	}

	pub fn current(&self) -> Option<Arc<StartScreen>> {
		self.owner.as_ref().map(|owner| owner.screen.clone())
	}

	/// Shows `screen` without saving it, as if a plugin had set it; fixture runs only.
	#[cfg(feature = "demo")]
	pub fn preview(&mut self, screen: StartScreen) {
		self.owner = Some(Owner {
			account: String::new(),
			plugin: "start-screen-demo".into(),
			screen: Arc::new(screen),
		});
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
				Some((_, Some(Some(screen)))) if *owner.screen != *screen => {
					next = Some(Owner {
						screen: Arc::new(screen.clone()),
						..owner.clone()
					});
				}
				Some(_) => {}
			}
		}
		let candidate = plugins
			.filter_map(|(id, screen)| Some((id, screen.as_ref()?.as_ref()?)))
			.min_by_key(|(id, _)| *id);
		if let Some((id, screen)) = candidate
			&& next.as_ref().is_none_or(|owner| {
				id < owner.plugin.as_str() || id == owner.plugin && owner.account != key
			}) {
			next = Some(Owner {
				account: key,
				plugin: id.to_owned(),
				screen: Arc::new(screen.clone()),
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
	}

	fn flush(&mut self) -> Option<String> {
		if self.writing.is_some() {
			return None;
		}
		let path = self.path.clone()?;
		let owner = self.unsaved.take()?;
		let context = self.context.clone();
		let (sender, receiver) = mpsc::sync_channel(1);
		let spawned = std::thread::Builder::new()
			.name("serein-start-screen-save".into())
			.spawn(move || {
				let _ = sender.send(write(&path, owner.as_ref()));
				if let Some(context) = context {
					context.request_repaint();
				}
			});
		match spawned {
			Ok(_) => {
				self.writing = Some(receiver);
				None
			}
			Err(_) => Some("Start screen could not be saved".into()),
		}
	}

	/// Finishes a completed write and starts the newest waiting one; returns a failure.
	/// Called once per frame.
	pub fn poll(&mut self) -> Option<String> {
		let finished = match self.writing.as_ref().map(mpsc::Receiver::try_recv) {
			Some(Err(TryRecvError::Empty)) => return None,
			Some(Ok(result)) => result.err(),
			Some(Err(TryRecvError::Disconnected)) => Some("Start screen could not be saved".into()),
			None => None,
		};
		self.writing = None;
		self.flush().or(finished)
	}

	/// Blocks until every queued write has finished.
	#[cfg(test)]
	pub fn settle(&mut self) {
		loop {
			assert_eq!(self.flush(), None);
			let Some(receiver) = self.writing.take() else {
				return;
			};
			assert_eq!(receiver.recv(), Ok(Ok(())));
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
		screen: &owner.screen,
	})
	.map_err(|_| "Cannot encode the start screen")?;
	if bytes.len() > MAX_FILE_BYTES {
		return Err("Start screen exceeds its device storage budget".into());
	}
	write_device_file(path, Some(&bytes))
}

/// Synthetic plugin result for `--demo-start-screen`: "Loading..." in pixel text under a
/// spinning spiky star (an icosahedron with a spike raised from every face), in the compact
/// splash window, with the SDK's default spin and pitch.
#[cfg(feature = "demo")]
pub fn demo_screen() -> StartScreen {
	const SPIKE: f32 = 1.9;
	let golden = (1.0 + 5f32.sqrt()) / 2.0;
	let unit = |[x, y, z]: [f32; 3]| {
		let length = (x * x + y * y + z * z).sqrt();
		[x / length, y / length, z / length]
	};
	let mut vertices: Vec<[f32; 3]> = [
		[-1.0, golden, 0.0],
		[1.0, golden, 0.0],
		[-1.0, -golden, 0.0],
		[1.0, -golden, 0.0],
		[0.0, -1.0, golden],
		[0.0, 1.0, golden],
		[0.0, -1.0, -golden],
		[0.0, 1.0, -golden],
		[golden, 0.0, -1.0],
		[golden, 0.0, 1.0],
		[-golden, 0.0, -1.0],
		[-golden, 0.0, 1.0],
	]
	.map(unit)
	.to_vec();
	let faces: [[u16; 3]; 20] = [
		[0, 11, 5],
		[0, 5, 1],
		[0, 1, 7],
		[0, 7, 10],
		[0, 10, 11],
		[1, 5, 9],
		[5, 11, 4],
		[11, 10, 2],
		[10, 7, 6],
		[7, 1, 8],
		[3, 9, 4],
		[3, 4, 2],
		[3, 2, 6],
		[3, 6, 8],
		[3, 8, 9],
		[4, 9, 5],
		[2, 4, 11],
		[6, 2, 10],
		[8, 6, 7],
		[9, 8, 1],
	];
	let mut triangles = Vec::with_capacity(faces.len() * 3);
	for [a, b, c] in faces {
		let [pa, pb, pc] = [a, b, c].map(|index| vertices[usize::from(index)]);
		let tip = unit(std::array::from_fn(|axis| pa[axis] + pb[axis] + pc[axis]));
		let index = vertices.len() as u16;
		vertices.push(tip.map(|value| value * SPIKE));
		triangles.extend([[a, b, index], [b, c, index], [c, a, index]]);
	}
	StartScreen {
		title: Some("Loading...".into()),
		subtitle: None,
		text_style: extensions::StartTextStyle::Dither,
		hide_logo: true,
		compact: true,
		model: Some(extensions::StartModel {
			vertices,
			triangles,
			color: None,
			spin: 12.0,
			pitch: 20.0,
		}),
	}
}
