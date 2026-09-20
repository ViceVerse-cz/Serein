//! User-invoked, bounded extension work. No package IO or Wasm runs on the UI thread.
use std::{
	collections::VecDeque,
	fs::{self, File, OpenOptions},
	io::{Cursor, Read, Write},
	net::{IpAddr, Ipv4Addr},
	path::{Path, PathBuf},
	sync::{
		Arc,
		atomic::{AtomicU64, Ordering},
		mpsc::{self, Receiver},
	},
	time::Duration,
};

use extensions::{
	Capability, Catalog, CatalogEntry, ExtensionKind, Invocation, Manifest, Output, Package,
	Preview, Surface, Theme,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const CATALOG_URL: &str =
	"https://raw.githubusercontent.com/ViceVerse-cz/rustcord/main/extensions/catalog.json";
const MAX_PACKAGE: usize = 16 * 1024 * 1024;
const MAX_RECORD: usize = MAX_PACKAGE + 64 * 1024;
const MAX_CATALOG: usize = 1024 * 1024;
const MAX_STORAGE: usize = 1024 * 1024;
const MAX_PER_SCOPE: usize = 8;
const MAX_ACCOUNTS: usize = 8;
const MAX_QUEUE: usize = 4;

#[derive(Clone)]
pub enum InstallSource {
	Bundled {
		bytes: &'static [u8],
		sha256: String,
		manifest: Manifest,
	},
	Catalog(CatalogEntry),
	Local {
		path: PathBuf,
		sha256: String,
		manifest: Manifest,
	},
}

pub enum Job {
	EditTheme {
		id: String,
		preview: bool,
	},
	SaveTheme {
		package: Box<Package>,
	},
	ExportTheme {
		package: Box<Package>,
		path: PathBuf,
	},
	SelectTheme {
		id: Option<String>,
	},
	Load {
		account: Option<String>,
	},
	RefreshCatalog {
		demo: bool,
	},
	Preview {
		id: String,
		preview: Preview,
		demo: bool,
	},
	InspectImport {
		path: PathBuf,
	},
	Enable {
		source: Box<InstallSource>,
		grants: Vec<Capability>,
		account: Option<String>,
	},
	Disable {
		id: String,
		kind: ExtensionKind,
		account: Option<String>,
	},
	Invoke {
		id: String,
		account: String,
		invocation: Invocation,
	},
	Logout {
		account: String,
	},
}

#[derive(Clone)]
pub struct Starter {
	pub source: InstallSource,
	pub theme: Option<Theme>,
	pub description: &'static str,
	pub download_bytes: u64,
}

pub(crate) fn starters() -> Result<Vec<Starter>, String> {
	let packages: [(&'static [u8], &'static str); 11] = [
		(
			include_bytes!(
				"../../../examples/extensions/packages/message-delete-protector.serein-extension"
			),
			"Keep messages already seen in this session visible in red after deletion. Cleared when disabled or signed out.",
		),
		(
			include_bytes!(
				"../../../examples/extensions/packages/emoji-sticker-images.serein-extension"
			),
			"While enabled, selecting custom emoji or stickers sends an image attachment immediately.",
		),
		(
			include_bytes!("../../../extensions/ocean.serein-extension"),
			"Deep blue surfaces with a bright ocean accent.",
		),
		(
			include_bytes!("../../../extensions/midnight.serein-extension"),
			"Inky midnight surfaces with a vivid violet accent.",
		),
		(
			include_bytes!("../../../extensions/rose.serein-extension"),
			"Soft rose surfaces with a warm pink accent.",
		),
		(
			include_bytes!("../../../extensions/forest.serein-extension"),
			"Calm forest greens and fresh leafy accents.",
		),
		(
			include_bytes!("../../../extensions/latte.serein-extension"),
			"Warm coffee tones and a creamy caramel accent.",
		),
		(
			include_bytes!("../../../extensions/golden.serein-extension"),
			"Warm charcoal and gold, with rounded, roomy controls.",
		),
		(
			include_bytes!("../../../extensions/katana.serein-extension"),
			"Katana's dark charcoal surfaces and sharp red accents. Light mode uses built-in colors.",
		),
		(
			include_bytes!("../../../extensions/obsidian.serein-extension"),
			"Obsidian violet surfaces and lavender accents in light and dark.",
		),
		(
			include_bytes!("../../../extensions/teal.serein-extension"),
			"Cool blue-green surfaces with fresh teal accents.",
		),
	];
	packages
		.into_iter()
		.map(|(bytes, description)| {
			let package = extensions::parse_package(bytes).map_err(|error| error.to_string())?;
			Ok(Starter {
				source: InstallSource::Bundled {
					bytes,
					sha256: digest(bytes),
					manifest: package.manifest,
				},
				theme: package.theme,
				description,
				download_bytes: bytes.len() as u64,
			})
		})
		.collect()
}

/// Offline debug smoke check of the exact starter packages embedded in the app.
#[cfg(feature = "demo")]
pub fn demo_check_examples() -> Result<bool, String> {
	let starters = starters()?;
	if starters.len() != 11
		|| starters
			.iter()
			.filter(|entry| entry.theme.is_some())
			.count() != 9
	{
		return Err("Expected two starter plugins and nine themes".into());
	}
	let gate = Gate {
		epoch: 0,
		generation: Arc::new(AtomicU64::new(0)),
		wake_cancel: Arc::new(tokio::sync::Notify::new()),
	};
	let mut activated = false;
	for starter in starters {
		let InstallSource::Bundled {
			bytes,
			sha256,
			manifest,
		} = starter.source
		else {
			return Err("Starter source must be bundled".into());
		};
		if digest(bytes) != sha256 || bytes.len() as u64 != starter.download_bytes {
			return Err("Starter checksum or byte length changed".into());
		}
		let package = extensions::parse_package(bytes).map_err(|error| error.to_string())?;
		if package.manifest != manifest {
			return Err("Starter manifest changed".into());
		}
		let mut stored = Stored {
			local_theme: false,
			grants: manifest.capabilities.clone(),
			package,
			reviewed: true,
			sha256,
			download_bytes: starter.download_bytes,
		};
		let summary = stored.summary(&gate, None, true);
		if let Some(error) = summary.error {
			return Err(error);
		}
		if manifest.kind == ExtensionKind::Plugin {
			if !(manifest.id == "message-delete-protector" && summary.preserve_deleted_messages
				|| manifest.id == "emoji-sticker-images" && summary.image_sharing)
			{
				return Err("Bundled plugin did not activate".into());
			}
			activated = true;
			stored.grants.clear();
			let denied = stored.summary(&gate, None, true);
			if denied.preserve_deleted_messages || denied.image_sharing || denied.error.is_none() {
				return Err("Plugin activation must require explicit permission".into());
			}
		} else if summary.preserve_deleted_messages
			|| summary.image_sharing
			|| summary.theme.is_none()
		{
			return Err("Theme starter must only supply a valid palette".into());
		}
	}
	let root = std::env::temp_dir().join(format!("serein-theme-check-{}", std::process::id()));
	fs::create_dir(&root).map_err(|_| "Cannot create isolated theme check directory")?;
	let result = (|| {
		for starter in self::starters()?
			.into_iter()
			.filter(|entry| entry.theme.is_some())
			.take(2)
		{
			enable(&root, starter.source, Vec::new(), None, &gate)?;
		}
		let installed = load(&root, None, &gate)?;
		assert_eq!(installed.len(), 2, "installing retains previous presets");
		let id = installed
			.iter()
			.find(|entry| !entry.active_theme)
			.unwrap()
			.manifest
			.id
			.clone();
		run(
			&root,
			Job::SelectTheme {
				id: Some(id.clone()),
			},
			&gate,
		)?;
		let reloaded = load(&root, None, &gate)?;
		assert_eq!(
			reloaded
				.iter()
				.find(|entry| entry.active_theme)
				.unwrap()
				.manifest
				.id,
			id
		);
		run(&root, Job::SelectTheme { id: None }, &gate)?;
		let reloaded = load(&root, None, &gate)?;
		assert_eq!(reloaded.len(), 2, "built-in presets keep installed themes");
		assert!(reloaded.iter().all(|entry| !entry.active_theme));
		Ok(activated)
	})();
	let cleanup =
		fs::remove_dir_all(&root).map_err(|_| "Cannot remove isolated theme check directory");
	cleanup?;
	result
}

#[derive(Clone)]
pub struct InstalledExtension {
	pub background_image: Option<Arc<eframe::egui::ColorImage>>,
	pub cover_image: Option<Arc<eframe::egui::ColorImage>>,
	pub local_theme: bool,
	pub active_theme: bool,
	pub manifest: Manifest,
	pub theme: Option<Theme>,
	pub reviewed: bool,
	pub sha256: String,
	pub download_bytes: u64,
	pub error: Option<String>,
	pub preserve_deleted_messages: bool,
	pub image_sharing: bool,
}

pub enum Event {
	ThemeSelected {
		id: Option<String>,
		background_image: Option<Arc<eframe::egui::ColorImage>>,
	},
	EditTheme {
		package: Box<Package>,
		image: Option<Arc<eframe::egui::ColorImage>>,
		cover: Option<Arc<eframe::egui::ColorImage>>,
		local_theme: bool,
		preview: bool,
	},
	ThemeExported,
	Loaded {
		installed: Vec<InstalledExtension>,
		starters: Vec<Starter>,
	},
	Catalog(Catalog),
	Preview {
		id: String,
		image: Option<eframe::egui::ColorImage>,
	},
	Imported {
		manifest: Manifest,
		source: InstallSource,
		download_bytes: u64,
	},
	Enabled(InstalledExtension),
	Disabled(String),
	Invoked {
		id: String,
		output: Output,
	},
	LoggedOut,
}

/// A single worker exists only while a job runs. Queued jobs and results have fixed limits.
pub struct ExtensionHost {
	root: PathBuf,
	generation: Arc<AtomicU64>,
	wake_cancel: Arc<tokio::sync::Notify>,
	queue: VecDeque<(u64, Job)>,
	next_token: u64,
	active: Option<(u64, Receiver<Result<Event, String>>)>,
	context: Option<eframe::egui::Context>,
}

impl ExtensionHost {
	pub fn new(root: PathBuf) -> Self {
		Self {
			root,
			generation: Arc::new(AtomicU64::new(0)),
			wake_cancel: Arc::new(tokio::sync::Notify::new()),
			queue: VecDeque::new(),
			next_token: 0,
			active: None,
			context: None,
		}
	}

	pub fn cancel(&mut self) {
		self.generation.fetch_add(1, Ordering::AcqRel);
		self.wake_cancel.notify_waiters();
		self.queue
			.retain(|(_, job)| matches!(job, Job::Disable { .. } | Job::Logout { .. }));
	}

	pub fn submit(&mut self, job: Job, context: &eframe::egui::Context) -> Result<u64, String> {
		if matches!(job, Job::Disable { .. } | Job::Logout { .. }) {
			self.cancel();
		}
		if self.queue.len() >= MAX_QUEUE {
			return Err("Extensions are busy; try again after the current operation".into());
		}
		validate_job(&job)?;
		self.context = Some(context.clone());
		let token = self.next_token;
		self.next_token = self.next_token.wrapping_add(1);
		self.queue.push_back((token, job));
		self.start_next();
		Ok(token)
	}

	pub fn busy(&self) -> bool {
		self.active.is_some() || !self.queue.is_empty()
	}

	pub fn poll(&mut self) -> Option<(u64, Result<Event, String>)> {
		let (token, receiver) = self.active.as_ref()?;
		let token = *token;
		let result = receiver.try_recv();
		match result {
			Ok(result) => {
				self.active = None;
				self.start_next();
				Some((token, result))
			}
			Err(mpsc::TryRecvError::Empty) => None,
			Err(mpsc::TryRecvError::Disconnected) => {
				self.active = None;
				self.start_next();
				Some((token, Err("Extension worker stopped unexpectedly".into())))
			}
		}
	}

	fn start_next(&mut self) {
		if self.active.is_some() {
			return;
		}
		let Some((token, job)) = self.queue.pop_front() else {
			return;
		};
		let (sender, receiver) = mpsc::sync_channel(1);
		let root = self.root.clone();
		let generation = if matches!(job, Job::Disable { .. } | Job::Logout { .. }) {
			Arc::new(AtomicU64::new(0))
		} else {
			self.generation.clone()
		};
		let gate = Gate {
			epoch: generation.load(Ordering::Acquire),
			generation,
			wake_cancel: self.wake_cancel.clone(),
		};
		let context = self.context.clone();
		self.active = Some((token, receiver));
		if std::thread::Builder::new()
			.name("serein-extension".into())
			.spawn(move || {
				let result = run(&root, job, &gate);
				let _ = sender.send(result);
				if let Some(context) = context {
					context.request_repaint();
				}
			})
			.is_err()
		{
			// The disconnected receiver reports the failure through the ordinary error UI.
			if let Some(context) = &self.context {
				context.request_repaint();
			}
		}
	}
}

impl Drop for ExtensionHost {
	fn drop(&mut self) {
		self.cancel();
	}
}

struct Gate {
	epoch: u64,
	generation: Arc<AtomicU64>,
	wake_cancel: Arc<tokio::sync::Notify>,
}
impl Gate {
	fn check(&self) -> Result<(), String> {
		if self.epoch == self.generation.load(Ordering::Acquire) {
			Ok(())
		} else {
			Err("Extension operation cancelled".into())
		}
	}
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Stored {
	#[serde(default)]
	local_theme: bool,
	package: Package,
	grants: Vec<Capability>,
	reviewed: bool,
	sha256: String,
	download_bytes: u64,
}

impl Stored {
	fn summary(&self, gate: &Gate, storage: Option<String>, active: bool) -> InstalledExtension {
		let activation = self
			.package
			.manifest
			.actions
			.iter()
			.find(|action| action.surface == Surface::Activation);
		let mut result = activation.map_or(Ok(extensions::Output::default()), |action| {
			validate_grants(&self.package.manifest, &self.grants)?;
			gate.check()?;
			let output = extensions::invoke(
				&self.package,
				&Invocation {
					action: action.id.clone(),
					storage,
					..Default::default()
				},
			)
			.map_err(|error| error.to_string())?;
			gate.check()?;
			if output.preserve_deleted_messages
				&& !self.grants.contains(&Capability::DeletedMessages)
			{
				return Err("Deleted message access was not granted".into());
			}
			if output.image_sharing && !self.grants.contains(&Capability::ImageSharing) {
				return Err("Image sharing access was not granted".into());
			}
			Ok(output)
		});
		let background_image = if active {
			match package_background(&self.package) {
				Ok(image) => image,
				Err(error) => {
					result = Err(error);
					None
				}
			}
		} else {
			None
		};
		let cover_image = match package_cover(&self.package) {
			Ok(image) => image,
			Err(error) => {
				result = Err(error);
				None
			}
		};
		InstalledExtension {
			background_image,
			cover_image,
			local_theme: self.local_theme,
			active_theme: self.package.manifest.kind == ExtensionKind::Theme,
			manifest: self.package.manifest.clone(),
			theme: self.package.theme.clone().or_else(|| {
				result
					.as_ref()
					.ok()
					.and_then(|output| output.appearance.clone())
			}),
			reviewed: self.reviewed,
			sha256: self.sha256.clone(),
			download_bytes: self.download_bytes,
			image_sharing: result.as_ref().is_ok_and(|output| output.image_sharing),
			preserve_deleted_messages: result
				.as_ref()
				.is_ok_and(|output| output.preserve_deleted_messages),
			error: result.err(),
		}
	}
}

fn validate_job(job: &Job) -> Result<(), String> {
	match job {
		Job::Invoke {
			id,
			account,
			invocation,
		} => {
			valid_id(id)?;
			account_key(account)?;
			if invocation.values.len() > 64
				|| invocation
					.action
					.len()
					.saturating_add(invocation.selected_message.as_ref().map_or(0, String::len))
					.saturating_add(invocation.composer.as_ref().map_or(0, String::len))
					.saturating_add(invocation.storage.as_ref().map_or(0, String::len))
					.saturating_add(
						invocation
							.values
							.iter()
							.map(|(key, value)| key.len().saturating_add(value.len()))
							.sum::<usize>(),
					) > 256 * 1024
			{
				return Err("Plugin input exceeds 256 KiB".into());
			}
		}
		Job::Preview { id, preview, .. } => {
			valid_id(id)?;
			preview.validate().map_err(|e| e.to_string())?;
		}
		Job::SelectTheme { id: Some(id) } | Job::Disable { id, .. } | Job::EditTheme { id, .. } => {
			valid_id(id)?
		}
		Job::SaveTheme { package } | Job::ExportTheme { package, .. } => {
			if package.manifest.kind != ExtensionKind::Theme {
				return Err("Theme editor only accepts themes".into());
			}
			package.validate().map_err(|error| error.to_string())?;
			if let Job::ExportTheme { path, .. } = job
				&& path.as_os_str().len() > 4096
			{
				return Err("Export path is too long".into());
			}
		}
		Job::Enable { grants, .. } if grants.len() > 4 => {
			return Err("Invalid plugin grants".into());
		}
		Job::InspectImport { path } if path.as_os_str().len() > 4096 => {
			return Err("Import path is too long".into());
		}
		_ => {}
	}
	Ok(())
}

fn run(root: &Path, job: Job, gate: &Gate) -> Result<Event, String> {
	gate.check()?;
	match job {
		Job::EditTheme { id, preview } => {
			valid_id(&id)?;
			let directory = root.join("themes").join(&id);
			if directory.with_extension("disabled").exists() {
				return Err("Theme cleanup is pending".into());
			}
			let (package, local_theme) = if directory.exists() {
				let stored = read_stored(&directory.join("package.json"))?;
				(stored.package, stored.local_theme)
			} else {
				let starter = starters()?
					.into_iter()
					.find(|starter| {
						matches!(&starter.source, InstallSource::Bundled { manifest, .. }
						if manifest.id == id && manifest.kind == ExtensionKind::Theme)
					})
					.ok_or("Theme is not available locally")?;
				let InstallSource::Bundled { bytes, .. } = starter.source else {
					return Err("Theme is not bundled".into());
				};
				(
					extensions::parse_package(bytes).map_err(|error| error.to_string())?,
					false,
				)
			};
			if package.manifest.id != id || package.manifest.kind != ExtensionKind::Theme {
				return Err("Theme does not match the requested package".into());
			}
			let image = package_background(&package)?;
			let cover = package_cover(&package)?;
			gate.check()?;
			Ok(Event::EditTheme {
				local_theme,
				preview,
				package: Box::new(package),
				image,
				cover,
			})
		}
		Job::SaveTheme { package } => {
			let bytes = encode_theme(&package)?;
			let _ = package_cover(&package)?;
			let directory = root.join("themes").join(&package.manifest.id);
			if directory.exists() && !read_stored(&directory.join("package.json"))?.local_theme {
				return Err(
					"This theme belongs to another package; duplicate it before saving".into(),
				);
			}
			let stored = Stored {
				package: *package,
				grants: Vec::new(),
				reviewed: false,
				local_theme: true,
				sha256: digest(&bytes),
				download_bytes: bytes.len() as u64,
			};
			install_stored(root, stored, None, gate).map(Event::Enabled)
		}
		Job::ExportTheme { package, path } => {
			let bytes = encode_theme(&package)?;
			// Reject invalid images before creating or replacing the user's export.
			let _ = package_background(&package)?;
			let _ = package_cover(&package)?;
			export_theme(&path, &bytes, gate)?;
			Ok(Event::ThemeExported)
		}
		Job::SelectTheme { id } => {
			let parent = root.join("themes");
			let mut background_image = None;
			if let Some(id) = &id {
				valid_id(id)?;
				let stored = read_stored(&parent.join(id).join("package.json"))?;
				if stored.package.manifest.id != *id
					|| stored.package.manifest.kind != ExtensionKind::Theme
					|| parent.join(id).with_extension("disabled").exists()
				{
					return Err("Theme is not installed".into());
				}
				background_image = package_background(&stored.package)?;
				atomic_write(&parent.join("active.json"), id.as_bytes(), gate)?;
			} else {
				remove_file(&parent.join("active.json"))?;
			}
			Ok(Event::ThemeSelected {
				id,
				background_image,
			})
		}
		Job::Load { account } => Ok(Event::Loaded {
			starters: starters()?,
			installed: load(root, account.as_deref(), gate)?,
		}),
		Job::RefreshCatalog { demo } => {
			let bytes = if demo {
				include_bytes!("../../../extensions/catalog.json").to_vec()
			} else {
				download(CATALOG_URL, MAX_CATALOG, gate, Duration::from_secs(60))?
			};
			extensions::parse_catalog(&bytes)
				.map(Event::Catalog)
				.map_err(|e| e.to_string())
		}
		Job::Preview { id, preview, demo } => {
			let image = load_preview(&id, &preview, demo, gate);
			Ok(Event::Preview { id, image })
		}
		Job::InspectImport { path } => {
			let bytes = read_bounded(&path, MAX_PACKAGE)?;
			let package = extensions::parse_package(&bytes).map_err(|e| e.to_string())?;
			let manifest = package.manifest;
			Ok(Event::Imported {
				source: InstallSource::Local {
					path,
					sha256: digest(&bytes),
					manifest: manifest.clone(),
				},
				manifest,
				download_bytes: bytes.len() as u64,
			})
		}
		Job::Enable {
			source,
			grants,
			account,
		} => enable(root, *source, grants, account.as_deref(), gate).map(Event::Enabled),
		Job::Disable { id, kind, account } => {
			let scope = scope(root, kind, account.as_deref())?;
			disable(&scope, &id, gate)?;
			Ok(Event::Disabled(id))
		}
		Job::Invoke {
			id,
			account,
			mut invocation,
		} => {
			let directory = scope(root, ExtensionKind::Plugin, Some(&account))?.join(&id);
			if directory.with_extension("disabled").exists() {
				return Err("Plugin is disabled".into());
			}
			let stored = read_stored(&directory.join("package.json"))?;
			if stored.package.manifest.id != id
				|| stored.package.manifest.kind != ExtensionKind::Plugin
			{
				return Err("Installed plugin identity changed".into());
			}
			if invocation.selected_message.is_some()
				&& !stored.grants.contains(&Capability::SelectedMessage)
				|| invocation.composer.is_some() && !stored.grants.contains(&Capability::Composer)
			{
				return Err("Plugin access was not granted".into());
			}
			invocation.storage = if stored.grants.contains(&Capability::Storage)
				&& directory.join("data.json").exists()
			{
				Some(
					String::from_utf8(read_bounded(&directory.join("data.json"), MAX_STORAGE)?)
						.map_err(|_| "Plugin data is invalid")?,
				)
			} else {
				None
			};
			gate.check()?;
			let mut output =
				extensions::invoke(&stored.package, &invocation).map_err(|e| e.to_string())?;
			gate.check()?;
			if let Some(data) = output.storage.take() {
				if !stored.grants.contains(&Capability::Storage) || data.len() > MAX_STORAGE {
					return Err("Plugin data exceeds its granted storage budget".into());
				}
				atomic_write(&directory.join("data.json"), data.as_bytes(), gate)?;
			}
			Ok(Event::Invoked { id, output })
		}
		Job::Logout { account } => {
			let directory = scope(root, ExtensionKind::Plugin, Some(&account))?;
			// The tombstone survives failed deletion and is retried before the next load.
			if directory.exists() {
				atomic_write(&directory.with_extension("disabled"), b"", gate)?;
				remove_owned_directory(&directory)?;
				remove_file(&directory.with_extension("disabled"))?;
			}
			Ok(Event::LoggedOut)
		}
	}
}

fn enable(
	root: &Path,
	source: InstallSource,
	grants: Vec<Capability>,
	account: Option<&str>,
	gate: &Gate,
) -> Result<InstalledExtension, String> {
	let (bytes, manifest, sha256, reviewed) = match source {
		InstallSource::Bundled {
			bytes,
			manifest,
			sha256,
		} => (bytes.to_vec(), manifest, sha256, true),
		InstallSource::Catalog(entry) => {
			let bytes = download(
				&entry.release_url,
				MAX_PACKAGE,
				gate,
				Duration::from_secs(60),
			)?;
			if bytes.len() as u64 != entry.download_bytes {
				return Err("Extension download size changed".into());
			}
			(bytes, entry.manifest, entry.sha256, true)
		}
		InstallSource::Local {
			path,
			sha256,
			manifest,
		} => (read_bounded(&path, MAX_PACKAGE)?, manifest, sha256, false),
	};
	if !digest(&bytes).eq_ignore_ascii_case(&sha256) {
		return Err("Extension package checksum changed; inspect the release again".into());
	}
	let package = extensions::parse_package(&bytes).map_err(|e| e.to_string())?;
	if package.manifest != manifest {
		return Err("Package does not match the reviewed manifest".into());
	}
	let stored = Stored {
		local_theme: false,
		package,
		grants,
		reviewed,
		sha256: sha256.to_ascii_lowercase(),
		download_bytes: bytes.len() as u64,
	};
	install_stored(root, stored, account, gate)
}

fn encode_theme(package: &Package) -> Result<Vec<u8>, String> {
	if package.manifest.kind != ExtensionKind::Theme {
		return Err("Theme editor only accepts themes".into());
	}
	package.validate().map_err(|error| error.to_string())?;
	let bytes = serde_json::to_vec(package).map_err(|_| "Cannot encode theme package")?;
	if bytes.len() > MAX_PACKAGE {
		return Err("Theme package exceeds 16 MiB".into());
	}
	Ok(bytes)
}

fn install_stored(
	root: &Path,
	stored: Stored,
	account: Option<&str>,
	gate: &Gate,
) -> Result<InstalledExtension, String> {
	validate_grants(&stored.package.manifest, &stored.grants)?;
	let parent = scope(root, stored.package.manifest.kind, account)?;
	let directory = parent.join(&stored.package.manifest.id);
	if parent.with_extension("disabled").exists() {
		return Err("Account extension cleanup is incomplete; reopen Extensions to retry".into());
	}
	if !directory.exists() && count_directories(&parent)? >= MAX_PER_SCOPE {
		return Err(
			"Disable an extension first; each scope allows eight installed extensions".into(),
		);
	}
	if stored.package.manifest.kind == ExtensionKind::Plugin
		&& !parent.exists()
		&& count_directories(&root.join("accounts"))? >= MAX_ACCOUNTS
	{
		return Err("Extension storage is full; log out an old account first".into());
	}
	let summary = stored.summary(gate, activation_storage(&stored, &directory)?, true);
	if let Some(error) = &summary.error {
		return Err(error.clone());
	}
	let record = serde_json::to_vec(&stored).map_err(|_| "Cannot encode extension package")?;
	if record.len() > MAX_RECORD {
		return Err("Extension package exceeds storage budget".into());
	}
	gate.check()?;
	fs::create_dir_all(&directory).map_err(|_| "Cannot create extension directory")?;
	atomic_write(&directory.join("package.json"), &record, gate)?;
	remove_file(&directory.with_extension("disabled"))?;
	if stored.package.manifest.kind == ExtensionKind::Theme {
		atomic_write(
			&parent.join("active.json"),
			stored.package.manifest.id.as_bytes(),
			gate,
		)?;
	}
	Ok(summary)
}

fn validate_grants(manifest: &Manifest, grants: &[Capability]) -> Result<(), String> {
	if grants.len() != manifest.capabilities.len()
		|| manifest
			.capabilities
			.iter()
			.any(|cap| !grants.contains(cap))
	{
		return Err("Confirm every requested capability before enabling this extension".into());
	}
	Ok(())
}

fn scope(root: &Path, kind: ExtensionKind, account: Option<&str>) -> Result<PathBuf, String> {
	match kind {
		ExtensionKind::Theme => Ok(root.join("themes")),
		ExtensionKind::Plugin => Ok(root.join("accounts").join(account_key(
			account.ok_or("Select an account before enabling plugins")?,
		)?)),
	}
}

fn account_key(account: &str) -> Result<String, String> {
	if account.is_empty() || account.len() > 128 {
		return Err("Invalid extension account scope".into());
	}
	Ok(digest(account.as_bytes()))
}

fn valid_id(id: &str) -> Result<(), String> {
	if !extensions::valid_id(id) {
		return Err("Invalid extension identifier".into());
	}
	Ok(())
}

fn load(
	root: &Path,
	account: Option<&str>,
	gate: &Gate,
) -> Result<Vec<InstalledExtension>, String> {
	cleanup(&root.join("accounts"), MAX_ACCOUNTS, gate)?;
	let mut scopes = vec![(root.join("themes"), ExtensionKind::Theme)];
	if let Some(account) = account {
		scopes.push((
			scope(root, ExtensionKind::Plugin, Some(account))?,
			ExtensionKind::Plugin,
		));
	}
	let mut installed = Vec::new();
	for (parent, kind) in scopes {
		let cleanup_error = cleanup(&parent, MAX_PER_SCOPE, gate).err();
		let mut active_theme = None;
		if kind == ExtensionKind::Theme && parent.join("active.json").exists() {
			let active = String::from_utf8(read_bounded(&parent.join("active.json"), 64)?)
				.map_err(|_| "Invalid active theme")?;
			valid_id(&active)?;
			active_theme = Some(active);
		}
		if !parent.exists() {
			continue;
		}
		for entry in fs::read_dir(&parent)
			.map_err(|_| "Cannot read extensions")?
			.take(MAX_PER_SCOPE * 4 + 1)
		{
			let entry = entry.map_err(|_| "Cannot read extension entry")?;
			if entry
				.file_type()
				.map_err(|_| "Cannot read extension entry")?
				.is_dir()
			{
				if installed.len() >= MAX_PER_SCOPE * 2 {
					return Err("Too many installed extensions".into());
				}
				let id = entry.file_name().to_string_lossy().into_owned();
				valid_id(&id)?;
				let package_path = entry.path().join("package.json");
				if !package_path.exists() {
					remove_owned_directory(&entry.path())?;
					continue;
				}
				let stored = read_stored(&package_path).and_then(|stored| {
					if entry.path().with_extension("disabled").exists() {
						return Err(cleanup_error.clone().unwrap_or_else(|| {
							"Extension is disabled; cleanup needs retrying".into()
						}));
					}
					if stored.package.manifest.id != id || stored.package.manifest.kind != kind {
						return Err("Installed extension identity changed".into());
					}
					Ok(stored)
				});
				remove_file(&entry.path().join("package.partial"))?;
				remove_file(&entry.path().join("data.partial"))?;
				installed.push(match stored {
					Ok(stored) => match activation_storage(&stored, &entry.path()) {
						Ok(storage) => {
							let mut summary = stored.summary(
								gate,
								storage,
								kind == ExtensionKind::Theme
									&& active_theme.as_deref() == Some(id.as_str()),
							);
							summary.active_theme = kind == ExtensionKind::Theme
								&& active_theme.as_deref() == Some(id.as_str());
							summary
						}
						Err(error) => {
							let mut summary = stored.summary(gate, None, false);
							summary.theme = None;
							summary.preserve_deleted_messages = false;
							summary.image_sharing = false;
							summary.error = Some(error);
							summary
						}
					},
					Err(error) => InstalledExtension {
						background_image: None,
						cover_image: None,
						local_theme: false,
						active_theme: false,
						manifest: Manifest {
							api_version: extensions::API_VERSION,
							id: id.clone(),
							name: id,
							version: "invalid".into(),
							author: "Unknown".into(),
							license: "Unknown".into(),
							source: "https://github.com/ViceVerse-cz/rustcord".into(),
							kind,
							capabilities: Vec::new(),
							actions: Vec::new(),
						},
						theme: None,
						reviewed: false,
						sha256: String::new(),
						download_bytes: 0,
						error: Some(error),
						preserve_deleted_messages: false,
						image_sharing: false,
					},
				});
			}
		}
	}
	Ok(installed)
}

fn activation_storage(stored: &Stored, directory: &Path) -> Result<Option<String>, String> {
	let path = directory.join("data.json");
	if stored.grants.contains(&Capability::Storage) && path.exists() {
		String::from_utf8(read_bounded(&path, MAX_STORAGE)?)
			.map(Some)
			.map_err(|_| "Plugin data is invalid".into())
	} else {
		Ok(None)
	}
}

fn read_stored(path: &Path) -> Result<Stored, String> {
	let bytes = read_bounded(path, MAX_RECORD)?;
	let stored: Stored =
		serde_json::from_slice(&bytes).map_err(|_| "Installed extension metadata is invalid")?;
	if stored.sha256.len() != 64
		|| !stored.sha256.bytes().all(|b| b.is_ascii_hexdigit())
		|| stored.download_bytes == 0
		|| stored.download_bytes > MAX_PACKAGE as u64
	{
		return Err("Installed extension fingerprint is invalid".into());
	}
	stored.package.validate().map_err(|e| e.to_string())?;
	validate_grants(&stored.package.manifest, &stored.grants)?;
	Ok(stored)
}

fn disable(parent: &Path, id: &str, gate: &Gate) -> Result<(), String> {
	valid_id(id)?;
	if !parent.exists() {
		return Ok(());
	}
	let directory = parent.join(id);
	atomic_write(&directory.with_extension("disabled"), b"", gate)?;
	remove_owned_directory(&directory)?;
	if parent.join("active.json").exists()
		&& read_bounded(&parent.join("active.json"), 64)? == id.as_bytes()
	{
		remove_file(&parent.join("active.json"))?;
	}
	remove_file(&directory.with_extension("disabled"))
}

fn cleanup(parent: &Path, max: usize, gate: &Gate) -> Result<(), String> {
	if !parent.exists() {
		return Ok(());
	}
	for entry in fs::read_dir(parent)
		.map_err(|_| "Cannot inspect extension cleanup")?
		.take(max * 4 + 1)
	{
		gate.check()?;
		let entry = entry.map_err(|_| "Cannot inspect extension cleanup")?;
		if entry
			.path()
			.extension()
			.is_some_and(|extension| extension == "disabled")
		{
			let id = entry
				.path()
				.file_stem()
				.ok_or("Invalid cleanup marker")?
				.to_string_lossy()
				.into_owned();
			valid_id(&id)?;
			remove_owned_directory(&parent.join(&id))?;
			remove_file(&entry.path())?;
		} else if entry
			.path()
			.extension()
			.is_some_and(|extension| extension == "partial")
		{
			remove_file(&entry.path())?;
		}
	}
	Ok(())
}

fn count_directories(path: &Path) -> Result<usize, String> {
	if !path.exists() {
		return Ok(0);
	}
	let mut count = 0;
	for entry in fs::read_dir(path)
		.map_err(|_| "Cannot inspect extension budget")?
		.take(MAX_PER_SCOPE * 4 + 1)
	{
		if entry
			.map_err(|_| "Cannot inspect extension budget")?
			.file_type()
			.map_err(|_| "Cannot inspect extension budget")?
			.is_dir()
		{
			count += 1;
		}
	}
	Ok(count)
}

fn read_bounded(path: &Path, limit: usize) -> Result<Vec<u8>, String> {
	let file = File::open(path).map_err(|_| "Cannot read extension file")?;
	if !file
		.metadata()
		.map_err(|_| "Cannot inspect extension file")?
		.is_file()
	{
		return Err("Extension package must be a regular file".into());
	}
	let mut bytes = Vec::new();
	file.take(limit as u64 + 1)
		.read_to_end(&mut bytes)
		.map_err(|_| "Cannot read extension file")?;
	if bytes.len() > limit {
		return Err("Extension file exceeds its byte budget".into());
	}
	Ok(bytes)
}

fn export_theme(path: &Path, bytes: &[u8], gate: &Gate) -> Result<(), String> {
	gate.check()?;
	let mut nonce = [0; 16];
	getrandom::fill(&mut nonce).map_err(|_| "Cannot prepare theme export")?;
	let temporary = path.with_extension(format!("{}.partial", digest(&nonce)));
	let mut file = OpenOptions::new()
		.write(true)
		.create_new(true)
		.open(&temporary)
		.map_err(|_| "Cannot create theme export")?;
	let result = (|| {
		file.write_all(bytes)
			.map_err(|_| "Cannot write theme export")?;
		file.sync_all().map_err(|_| "Cannot finish theme export")?;
		drop(file);
		gate.check()?;
		fs::rename(&temporary, path).map_err(|_| "Cannot replace theme export".to_owned())
	})();
	if result.is_err() {
		let _ = remove_file(&temporary);
	}
	result
}

fn atomic_write(path: &Path, bytes: &[u8], gate: &Gate) -> Result<(), String> {
	gate.check()?;
	let temporary = path.with_extension("partial");
	let result: Result<(), String> = (|| {
		let mut options = OpenOptions::new();
		options.write(true).create(true).truncate(true);
		#[cfg(unix)]
		{
			use std::os::unix::fs::OpenOptionsExt;
			options.mode(0o600);
		}
		let mut file = options
			.open(&temporary)
			.map_err(|_| "Cannot write extension data")?;
		file.write_all(bytes)
			.map_err(|_| "Cannot write extension data")?;
		file.sync_all()
			.map_err(|_| "Cannot finish extension data")?;
		drop(file);
		gate.check()?;
		fs::rename(&temporary, path).map_err(|_| "Cannot replace extension data".to_owned())
	})();
	if result.is_err() {
		let _ = remove_file(&temporary);
	}
	result
}

fn remove_file(path: &Path) -> Result<(), String> {
	match fs::remove_file(path) {
		Ok(()) => Ok(()),
		Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
		Err(_) => Err("Extension cleanup failed; retry or reopen Extensions".into()),
	}
}

fn remove_owned_directory(path: &Path) -> Result<(), String> {
	match fs::symlink_metadata(path) {
		Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
		Err(_) => return Err("Cannot inspect extension cleanup path".into()),
		Ok(metadata) if metadata.file_type().is_symlink() => {
			return Err("Extension directory cannot be a symbolic link".into());
		}
		Ok(_) => {}
	}
	fs::remove_dir_all(path).map_err(|_| "Extension cleanup failed; it will retry on launch".into())
}

fn load_preview(
	id: &str,
	preview: &Preview,
	demo: bool,
	gate: &Gate,
) -> Option<eframe::egui::ColorImage> {
	gate.check().ok()?;
	preview.validate().ok()?;
	let bytes = if demo {
		demo_preview(id)?.to_vec()
	} else {
		download(
			&preview.url,
			preview.download_bytes as usize,
			gate,
			Duration::from_secs(5),
		)
		.ok()?
	};
	gate.check().ok()?;
	let image = decode_preview(&bytes, preview)?;
	gate.check().ok()?;
	Some(image)
}

fn demo_preview(id: &str) -> Option<&'static [u8]> {
	#[cfg(feature = "demo")]
	{
		match id {
			"serein-ocean" => Some(include_bytes!("../../../extensions/previews/ocean.png")),
			_ => None,
		}
	}
	#[cfg(not(feature = "demo"))]
	{
		let _ = id;
		None
	}
}

pub fn read_background(path: &Path) -> Result<(Vec<u8>, eframe::egui::ColorImage), String> {
	let bytes = read_bounded(path, extensions::MAX_BACKGROUND_BYTES)?;
	let image = decode_background(&bytes)?;
	Ok((bytes, image))
}

pub fn read_cover(path: &Path) -> Result<(Vec<u8>, eframe::egui::ColorImage), String> {
	let bytes = read_bounded(path, extensions::MAX_BACKGROUND_BYTES)?;
	let image = decode_cover(&bytes)?;
	Ok((bytes, image))
}

fn decode_cover(bytes: &[u8]) -> Result<eframe::egui::ColorImage, String> {
	if bytes.len() > extensions::MAX_BACKGROUND_BYTES {
		return Err("Cover image exceeds 2 MiB".into());
	}
	let image = decode_static_image(bytes, 4_000_000)
		.ok_or("Choose a static PNG or JPEG within 4096 pixels per edge and 4 million pixels")?
		.thumbnail(640, 360)
		.into_rgba8();
	Ok(eframe::egui::ColorImage::from_rgba_unmultiplied(
		[image.width() as usize, image.height() as usize],
		image.as_raw(),
	))
}

fn package_cover(package: &Package) -> Result<Option<Arc<eframe::egui::ColorImage>>, String> {
	if package.cover_image.is_empty() {
		Ok(None)
	} else {
		decode_cover(&package.cover_image).map(|image| Some(Arc::new(image)))
	}
}

pub fn decode_background(bytes: &[u8]) -> Result<eframe::egui::ColorImage, String> {
	if bytes.len() > extensions::MAX_BACKGROUND_BYTES {
		return Err("Background image exceeds 2 MiB".into());
	}
	let image = decode_static_image(bytes, 4_000_000)
		.ok_or("Choose a static PNG or JPEG within 4096 pixels per edge and 4 million pixels")?
		.into_rgba8();
	Ok(eframe::egui::ColorImage::from_rgba_unmultiplied(
		[image.width() as usize, image.height() as usize],
		image.as_raw(),
	))
}

fn package_background(package: &Package) -> Result<Option<Arc<eframe::egui::ColorImage>>, String> {
	if package.background_image.is_empty() {
		Ok(None)
	} else {
		decode_background(&package.background_image).map(|image| Some(Arc::new(image)))
	}
}

fn decode_static_image(bytes: &[u8], max_pixels: u64) -> Option<image::DynamicImage> {
	use image::ImageDecoder;
	let format = image::guess_format(bytes).ok()?;
	if !matches!(format, image::ImageFormat::Png | image::ImageFormat::Jpeg) {
		return None;
	}
	let mut reader = image::ImageReader::with_format(Cursor::new(bytes), format);
	let mut limits = image::Limits::default();
	limits.max_image_width = Some(4096);
	limits.max_image_height = Some(4096);
	limits.max_alloc = Some(32 * 1024 * 1024);
	reader.limits(limits);
	let decoder = reader.into_decoder().ok()?;
	let (width, height) = decoder.dimensions();
	if width == 0
		|| height == 0
		|| width > 4096
		|| height > 4096
		|| u64::from(width) * u64::from(height) > max_pixels
		|| decoder.total_bytes() > 32 * 1024 * 1024
	{
		return None;
	}
	image::DynamicImage::from_decoder(decoder).ok()
}

fn decode_preview(bytes: &[u8], preview: &Preview) -> Option<eframe::egui::ColorImage> {
	if bytes.len() > extensions::MAX_PREVIEW_BYTES
		|| bytes.len() as u64 != preview.download_bytes
		|| !digest(bytes).eq_ignore_ascii_case(&preview.sha256)
	{
		return None;
	}
	let mut image = decode_static_image(bytes, 4 * 1024 * 1024)?;
	if image.width() > 640 || image.height() > 360 {
		image = image.thumbnail(640, 360);
	}
	let image = image.into_rgba8();
	Some(eframe::egui::ColorImage::from_rgba_unmultiplied(
		[image.width() as usize, image.height() as usize],
		image.as_raw(),
	))
}

fn digest(bytes: &[u8]) -> String {
	format!("{:x}", Sha256::digest(bytes))
}

fn download(raw: &str, limit: usize, gate: &Gate, timeout: Duration) -> Result<Vec<u8>, String> {
	tokio::runtime::Builder::new_current_thread()
		.enable_all()
		.build()
		.map_err(|_| "Extension downloader unavailable")?
		.block_on(async {
			tokio::select! {
				biased;
				_ = gate.wake_cancel.notified() => Err("Extension operation cancelled".into()),
				result = tokio::time::timeout(timeout, download_https(raw, limit, gate)) => result.map_err(|_| "Extension download timed out")?,
			}
		})
}

async fn download_https(raw: &str, limit: usize, gate: &Gate) -> Result<Vec<u8>, String> {
	let mut url = url::Url::parse(raw).map_err(|_| "Invalid extension download URL")?;
	for _ in 0..6 {
		gate.check()?;
		let host = url.host_str().ok_or("Invalid extension download host")?;
		if url.scheme() != "https"
			|| !url.username().is_empty()
			|| url.password().is_some()
			|| url.port_or_known_default() != Some(443)
			|| !host.contains('.')
		{
			return Err("Extensions require credential-free public HTTPS URLs".into());
		}
		let addresses: Vec<_> = tokio::net::lookup_host((host, 443))
			.await
			.map_err(|_| "Extension download DNS failed")?
			.take(16)
			.collect();
		if addresses.is_empty() || addresses.iter().any(|address| !public_ip(address.ip())) {
			return Err("Extension downloads cannot access private networks".into());
		}
		// Pin the validated DNS result; redirects receive a fresh validation and client.
		let client = reqwest::Client::builder()
			.no_proxy()
			.no_gzip()
			.redirect(reqwest::redirect::Policy::none())
			.resolve_to_addrs(host, &addresses)
			.timeout(Duration::from_secs(30))
			.build()
			.map_err(|_| "Extension downloader unavailable")?;
		let mut response = client
			.get(url.clone())
			.header(reqwest::header::ACCEPT_ENCODING, "identity")
			.send()
			.await
			.map_err(|_| "Extension download failed")?;
		if response.status().is_redirection() {
			let location = response
				.headers()
				.get(reqwest::header::LOCATION)
				.and_then(|v| v.to_str().ok())
				.ok_or("Invalid extension redirect")?;
			url = url
				.join(location)
				.map_err(|_| "Invalid extension redirect")?;
			continue;
		}
		if response.status() != reqwest::StatusCode::OK {
			return Err(
				"Extension download unavailable; refresh the catalog or use a local package".into(),
			);
		}
		if response
			.content_length()
			.is_some_and(|size| size > limit as u64)
			|| response
				.headers()
				.get(reqwest::header::CONTENT_ENCODING)
				.is_some_and(|encoding| encoding != "identity")
		{
			return Err("Extension download size or encoding is unsupported".into());
		}
		let mut bytes = Vec::new();
		while let Some(chunk) = response
			.chunk()
			.await
			.map_err(|_| "Extension download interrupted")?
		{
			gate.check()?;
			if bytes.len().saturating_add(chunk.len()) > limit {
				return Err("Extension download exceeds its byte budget".into());
			}
			bytes.extend_from_slice(&chunk);
		}
		return Ok(bytes);
	}
	Err("Too many extension download redirects".into())
}

fn public_ip(ip: IpAddr) -> bool {
	match ip {
		IpAddr::V4(ip) => {
			let [a, b, _, _] = ip.octets();
			!ip.is_private()
				&& !ip.is_loopback()
				&& !ip.is_link_local()
				&& !ip.is_broadcast()
				&& !ip.is_documentation()
				&& !ip.is_multicast()
				&& a != 0 && a < 240
				&& !(a == 100 && (64..=127).contains(&b))
				&& !(a == 198 && (b == 18 || b == 19))
				&& !(a == 192 && b == 0)
				&& ip != Ipv4Addr::UNSPECIFIED
		}
		IpAddr::V6(ip) => {
			let segments = ip.segments();
			segments[0] & 0xe000 == 0x2000
				&& !(segments[0] == 0x2001 && (segments[1] == 0xdb8 || segments[1] < 0x200))
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	struct Profile(PathBuf);
	impl Profile {
		fn new() -> Self {
			let mut nonce = [0; 16];
			getrandom::fill(&mut nonce).unwrap();
			let path =
				std::env::temp_dir().join(format!("serein-extension-test-{}", digest(&nonce)));
			fs::create_dir(&path).unwrap();
			Self(path)
		}
	}
	impl Drop for Profile {
		fn drop(&mut self) {
			fs::remove_dir_all(&self.0).unwrap();
		}
	}
	fn gate() -> Gate {
		Gate {
			epoch: 0,
			generation: Arc::new(AtomicU64::new(0)),
			wake_cancel: Arc::new(tokio::sync::Notify::new()),
		}
	}
	#[test]
	fn bundled_themes_open_without_installing_and_preserve_preview_intent() {
		let profile = Profile::new();
		let root = profile.0.join("extensions");
		for starter in starters().unwrap() {
			let InstallSource::Bundled { manifest, .. } = starter.source else {
				unreachable!()
			};
			let result = run(
				&root,
				Job::EditTheme {
					id: manifest.id.clone(),
					preview: true,
				},
				&gate(),
			);
			if manifest.kind == ExtensionKind::Theme {
				let Event::EditTheme {
					package,
					local_theme,
					preview,
					..
				} = result.unwrap()
				else {
					panic!("expected theme")
				};
				assert_eq!(package.manifest.id, manifest.id);
				assert!(package.theme.is_some());
				assert!(preview && !local_theme);
			} else {
				assert!(result.is_err());
			}
		}
		assert!(!root.exists(), "preview must not install a theme");
		let original =
			extensions::parse_package(include_bytes!("../../../extensions/ocean.serein-extension"))
				.unwrap();
		let directory = root.join("themes").join(&original.manifest.id);
		fs::create_dir_all(&directory).unwrap();
		fs::write(directory.join("package.json"), b"invalid").unwrap();
		assert!(
			run(
				&root,
				Job::EditTheme {
					id: original.manifest.id,
					preview: false
				},
				&gate()
			)
			.is_err(),
			"do not hide a corrupt installed package behind the bundled original"
		);
	}
	fn theme(id: &str) -> Package {
		Package {
			manifest: Manifest {
				api_version: 1,
				id: id.into(),
				name: id.into(),
				version: "1.0.0".into(),
				author: "Synthetic test".into(),
				license: "MIT".into(),
				source: "https://github.com/ViceVerse-cz/rustcord".into(),
				kind: ExtensionKind::Theme,
				capabilities: Vec::new(),
				actions: Vec::new(),
			},
			theme: Some(Theme::default()),
			background_image: Vec::new(),
			cover_image: Vec::new(),
			wasm: Vec::new(),
		}
	}
	fn source(profile: &Profile, package: &Package) -> InstallSource {
		let bytes = serde_json::to_vec(package).unwrap();
		let path = profile.0.join(format!("{}.json", package.manifest.id));
		fs::write(&path, &bytes).unwrap();
		InstallSource::Local {
			path,
			sha256: digest(&bytes),
			manifest: package.manifest.clone(),
		}
	}

	#[test]
	fn local_theme_cover_survives_edit_without_another_copy() {
		let profile = Profile::new();
		let root = profile.0.join("extensions");
		let mut package = theme("local-cover");
		package.cover_image = include_bytes!("../../../extensions/previews/ocean.png").to_vec();
		let Event::Enabled(first) = run(
			&root,
			Job::SaveTheme {
				package: Box::new(package),
			},
			&gate(),
		)
		.unwrap() else {
			panic!("theme save did not install");
		};
		assert!(first.local_theme && first.cover_image.is_some());
		let Event::EditTheme {
			mut package,
			local_theme,
			cover,
			..
		} = run(
			&root,
			Job::EditTheme {
				id: "local-cover".into(),
				preview: false,
			},
			&gate(),
		)
		.unwrap()
		else {
			panic!("installed theme did not open");
		};
		assert!(local_theme && cover.is_some());
		package.manifest.name = "Updated cover".into();
		let Event::Enabled(updated) = run(&root, Job::SaveTheme { package }, &gate()).unwrap()
		else {
			panic!("theme edit did not save");
		};
		assert_eq!(updated.manifest.id, "local-cover");
		assert_eq!(updated.manifest.name, "Updated cover");
		assert!(updated.cover_image.is_some());
		assert_eq!(count_directories(&root.join("themes")).unwrap(), 1);

		let imported = source(&profile, &theme("imported"));
		enable(&root, imported, Vec::new(), None, &gate()).unwrap();
		assert!(
			run(
				&root,
				Job::SaveTheme {
					package: Box::new(theme("imported"))
				},
				&gate()
			)
			.is_err()
		);
	}

	#[test]
	fn theme_install_select_disable_restart_and_checksum() {
		let profile = Profile::new();
		let root = profile.0.join("extensions");
		let first = source(&profile, &theme("first"));
		let enabled = enable(&root, first.clone(), Vec::new(), None, &gate()).unwrap();
		let original = fs::read(profile.0.join("first.json")).unwrap();
		assert_eq!(enabled.sha256, digest(&original));
		assert_eq!(enabled.download_bytes, original.len() as u64);
		let reloaded = load(&root, None, &gate()).unwrap();
		assert_eq!(reloaded[0].sha256, enabled.sha256);
		assert_eq!(reloaded[0].download_bytes, enabled.download_bytes);
		assert_eq!(load(&root, None, &gate()).unwrap()[0].manifest.id, "first");
		let second = source(&profile, &theme("second"));
		enable(&root, second, Vec::new(), None, &gate()).unwrap();
		assert!(root.join("themes/first").exists());
		let installed = load(&root, None, &gate()).unwrap();
		assert_eq!(installed.len(), 2);
		assert_eq!(
			installed
				.iter()
				.find(|entry| entry.active_theme)
				.unwrap()
				.manifest
				.id,
			"second"
		);
		run(
			&root,
			Job::SelectTheme {
				id: Some("first".into()),
			},
			&gate(),
		)
		.unwrap();
		assert_eq!(
			load(&root, None, &gate())
				.unwrap()
				.iter()
				.find(|entry| entry.active_theme)
				.unwrap()
				.manifest
				.id,
			"first"
		);
		run(&root, Job::SelectTheme { id: None }, &gate()).unwrap();
		assert!(
			load(&root, None, &gate())
				.unwrap()
				.iter()
				.all(|entry| !entry.active_theme)
		);
		disable(&root.join("themes"), "second", &gate()).unwrap();
		assert_eq!(load(&root, None, &gate()).unwrap().len(), 1);
		assert!(!root.join("themes/second").exists());
		assert!(!root.join("themes/active.json").exists());
		if let InstallSource::Local { path, .. } = &first {
			fs::write(path, b"{}").unwrap();
		}
		assert!(enable(&root, first, Vec::new(), None, &gate()).is_err());
		assert!(
			profile.0.join("first.json").exists(),
			"The user's imported original survives disable"
		);
	}

	#[test]
	fn cleanup_retries_tombstones_and_accounts_remain_isolated() {
		let profile = Profile::new();
		let root = profile.0.join("extensions");
		let first = scope(&root, ExtensionKind::Plugin, Some("account-one")).unwrap();
		let second = scope(&root, ExtensionKind::Plugin, Some("account-two")).unwrap();
		fs::create_dir_all(first.join("plugin")).unwrap();
		fs::create_dir_all(second.join("plugin")).unwrap();
		fs::write(first.join("plugin/data.json"), b"private-one").unwrap();
		fs::write(second.join("plugin/data.json"), b"private-two").unwrap();
		fs::write(first.with_extension("disabled"), b"").unwrap();
		cleanup(&root.join("accounts"), MAX_ACCOUNTS, &gate()).unwrap();
		assert!(!first.exists());
		assert_eq!(
			fs::read(second.join("plugin/data.json")).unwrap(),
			b"private-two"
		);
		fs::write(second.join("plugin.disabled"), b"").unwrap();
		fs::write(second.join("orphan.partial"), b"unfinished").unwrap();
		cleanup(&second, MAX_PER_SCOPE, &gate()).unwrap();
		assert!(!second.join("plugin").exists());
		assert!(!second.join("orphan.partial").exists());
		assert!(scope(&root, ExtensionKind::Plugin, None).is_err());
		assert!(disable(&second, "../outside", &gate()).is_err());
	}

	#[test]
	fn invalid_package_remains_disableable_and_cancel_preserves_cleanup() {
		let profile = Profile::new();
		let root = profile.0.join("extensions");
		let directory = scope(&root, ExtensionKind::Plugin, Some("account")).unwrap();
		fs::create_dir_all(directory.join("broken")).unwrap();
		fs::write(directory.join("broken/package.json"), b"invalid").unwrap();
		let loaded = load(&root, Some("account"), &gate()).unwrap();
		assert_eq!(loaded.len(), 1);
		assert!(loaded[0].error.is_some());
		disable(&directory, "broken", &gate()).unwrap();
		let mut host = ExtensionHost::new(root);
		host.queue
			.push_back((0, Job::RefreshCatalog { demo: false }));
		host.queue.push_back((
			1,
			Job::Logout {
				account: "account".into(),
			},
		));
		host.cancel();
		assert_eq!(host.queue.len(), 1);
		assert!(matches!(host.queue.front(), Some((1, Job::Logout { .. }))));
		let cancelled = gate();
		cancelled.generation.fetch_add(1, Ordering::Release);
		assert!(atomic_write(&profile.0.join("cancelled.json"), b"no", &cancelled).is_err());
		assert!(!profile.0.join("cancelled.json").exists());
	}

	#[test]
	fn shop_preview_demo_catalog_and_images_are_local_and_hash_pinned() {
		let starters = starters().unwrap();
		assert_eq!(starters.len(), 11);
		let mut ids = std::collections::BTreeSet::new();
		for starter in starters {
			let InstallSource::Bundled {
				bytes,
				sha256,
				manifest,
			} = starter.source
			else {
				panic!("Expected bundled source");
			};
			assert_eq!(sha256, digest(bytes));
			assert!(ids.insert(manifest.id));
			assert_eq!(starter.download_bytes, bytes.len() as u64);
			assert_eq!(
				starter.theme.is_some(),
				manifest.kind == ExtensionKind::Theme
			);
		}
	}

	#[test]
	fn shop_preview_checks_hash_size_format_and_decode_bounds() {
		fn encoded(width: u32, height: u32, format: image::ImageFormat) -> Vec<u8> {
			let mut output = Cursor::new(Vec::new());
			image::DynamicImage::new_rgb8(width, height)
				.write_to(&mut output, format)
				.unwrap();
			output.into_inner()
		}
		fn metadata(bytes: &[u8]) -> Preview {
			Preview {
				url: "https://example.org/preview.png".into(),
				sha256: digest(bytes),
				download_bytes: bytes.len() as u64,
			}
		}
		for format in [image::ImageFormat::Png, image::ImageFormat::Jpeg] {
			let bytes = encoded(1280, 720, format);
			let mut preview = metadata(&bytes);
			assert_eq!(decode_preview(&bytes, &preview).unwrap().size, [640, 360]);
			preview.sha256 = "0".repeat(64);
			assert!(decode_preview(&bytes, &preview).is_none());
			preview = metadata(&bytes);
			preview.download_bytes += 1;
			assert!(decode_preview(&bytes, &preview).is_none());
		}
		for (width, height) in [(4097, 1), (2049, 2048)] {
			let bytes = encoded(width, height, image::ImageFormat::Png);
			assert!(decode_preview(&bytes, &metadata(&bytes)).is_none());
		}
		let gif = encoded(1, 1, image::ImageFormat::Gif);
		assert!(decode_preview(&gif, &metadata(&gif)).is_none());
		let too_large = vec![0; extensions::MAX_PREVIEW_BYTES + 1];
		assert!(decode_preview(&too_large, &metadata(&too_large)).is_none());
		let cancelled = gate();
		cancelled.generation.fetch_add(1, Ordering::Release);
		assert!(load_preview("synthetic", &metadata(b"bad"), false, &cancelled).is_none());
	}

	#[test]
	fn byte_limits_and_private_download_destinations_fail_closed() {
		let profile = Profile::new();
		let file = profile.0.join("large.json");
		fs::write(&file, b"12345").unwrap();
		assert!(read_bounded(&file, 4).is_err());
		for ip in [
			"127.0.0.1",
			"10.0.0.1",
			"192.168.0.1",
			"169.254.169.254",
			"100.64.0.1",
			"0.0.0.0",
			"::1",
			"fc00::1",
			"fe80::1",
			"::ffff:127.0.0.1",
			"2001:db8::1",
		] {
			assert!(!public_ip(ip.parse().unwrap()), "{ip}");
		}
		for ip in ["1.1.1.1", "2606:4700:4700::1111"] {
			assert!(public_ip(ip.parse().unwrap()), "{ip}");
		}
		let cancelled = gate();
		cancelled.generation.fetch_add(1, Ordering::Release);
		assert!(
			download(
				"https://github.com/example",
				100,
				&cancelled,
				Duration::from_secs(60)
			)
			.is_err()
		);
	}
}
