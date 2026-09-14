//! Credential-free release updates. Work and filesystem access stay off the UI thread.
use eframe::egui;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
	path::PathBuf,
	sync::{
		Arc,
		atomic::{AtomicBool, AtomicU64, Ordering},
		mpsc,
	},
	time::{Duration, Instant},
};
use tokio::runtime::Runtime;

#[path = "updater_install.rs"]
mod install;

const RELEASES: &str = "https://api.github.com/repos/ViceVerse-cz/Serein/releases";
const MAX_METADATA: usize = 2 * 1024 * 1024;
const MAX_DOWNLOAD: u64 = 512 * 1024 * 1024;
const CHECK_INTERVAL: Duration = Duration::from_secs(6 * 60 * 60);

#[derive(Clone, Deserialize)]
struct Asset {
	name: String,
	browser_download_url: String,
	size: u64,
}
#[derive(Clone, Deserialize)]
struct Release {
	tag_name: String,
	draft: bool,
	prerelease: bool,
	assets: Vec<Asset>,
}
#[derive(Clone)]
struct Package {
	version: String,
	// `None` on an installation with no in-app installer: the version is still reported,
	// but there is nothing here to download.
	archive: Option<Asset>,
	checksums: Option<Asset>,
}
struct Staged {
	directory: PathBuf,
	installation: PathBuf,
}
enum Outcome {
	Checked(Option<Package>),
	Downloaded(Staged),
	Prepared(install::Prepared),
	Cleaned,
}
struct Job {
	receiver: mpsc::Receiver<Result<Outcome, String>>,
	cancel: Arc<AtomicBool>,
	progress: Arc<AtomicU64>,
	total: u64,
}
pub struct Updater {
	demo: bool,
	channel: Option<bool>,
	next_check: Instant,
	last_check: Option<Instant>,
	job: Option<Job>,
	package: Option<Package>,
	staged: Option<Staged>,
	cleanup_pending: Option<Staged>,
	status: String,
	armed: bool,
	helper: Option<install::Prepared>,
	close_requested: bool,
	auto_download: bool,
	demo_available: bool,
	demo_ready: bool,
}
impl Updater {
	pub fn new(demo: bool) -> Self {
		Self {
			demo,
			channel: None,
			next_check: Instant::now(),
			last_check: None,
			job: None,
			package: None,
			staged: None,
			cleanup_pending: None,
			status: "Updates have not been checked yet.".into(),
			armed: false,
			helper: None,
			close_requested: false,
			auto_download: false,
			demo_available: false,
			demo_ready: false,
		}
	}

	/// Returns true only when a prepared restart should enter the normal close/draft gate.
	pub fn sync(
		&mut self,
		ctx: &egui::Context,
		runtime: &Runtime,
		view: &mut ui::updates::Updates,
		enabled: bool,
	) -> bool {
		let check = std::mem::take(&mut view.check_requested);
		let download = std::mem::take(&mut view.download_requested);
		let restart = std::mem::take(&mut view.restart_requested);
		if self.demo {
			if self.channel != Some(view.nightly) {
				self.demo_available = false;
				self.demo_ready = false;
			}
			self.channel = Some(view.nightly);
			if check {
				self.demo_available = true;
				self.status =
					"Synthetic preview: Serein 99.0.0 is available. No network request was made."
						.into();
			}
			if download && self.demo_available {
				self.demo_ready = true;
				self.status =
					"Synthetic preview: update ready to restart. No files were downloaded.".into();
			}
			if restart && self.demo_ready {
				self.status =
					"Synthetic preview: restart simulated. No installation was changed.".into();
			}
			view.status.clone_from(&self.status);
			view.available = self.demo_available;
			view.ready = self.demo_ready;
			view.busy = false;
			view.progress = None;
			view.supported = true;
			return false;
		}
		let supported = cfg!(any(target_os = "macos", windows)) || install::appimage_session();
		view.supported = supported;
		view.flatpak = install::flatpak_session();
		if !enabled {
			if let Some(job) = &self.job {
				job.cancel.store(true, Ordering::Relaxed);
			}
			view.busy = self.job.is_some();
			view.available = false;
			view.ready = false;
			view.progress = None;
			view.status =
				"Load update preferences or choose your update settings to enable checking.".into();
			return false;
		}
		if self.channel != Some(view.nightly) {
			if let Some(job) = &self.job {
				job.cancel.store(true, Ordering::Relaxed);
			}
			self.channel = Some(view.nightly);
			self.package = None;
			self.auto_download = false;
			self.cancel_restart();
			self.discard_stage();
			self.next_check = Instant::now();
			self.last_check = None;
			self.status = "Updates have not been checked on this channel yet.".into();
		}
		if let Some(job) = &self.job {
			match job.receiver.try_recv() {
				Ok(result) => {
					let cancelled = job.cancel.load(Ordering::Relaxed);
					self.job = None;
					if cancelled {
						match result {
							Ok(Outcome::Downloaded(stage)) => {
								self.cleanup_pending = Some(stage);
							}
							Ok(Outcome::Prepared(helper)) => {
								self.helper = Some(helper);
							}
							_ => {}
						}
					} else {
						match result {
							Ok(Outcome::Checked(package)) => {
								self.status = package.as_ref().map_or_else(
									|| "Serein is up to date on this channel.".into(),
									|p| {
										if install::flatpak_session() {
											format!(
												"Serein {} is available. Update with `flatpak update` or your Software center.",
												p.version,
											)
										} else {
											format!("Serein {} is available.", p.version)
										}
									},
								);
								self.package = package;
								self.auto_download = true;
							}
							Ok(Outcome::Downloaded(stage)) => {
								self.staged = Some(stage);
								self.status =
									"Update downloaded and verified. Restart when you are ready."
										.into();
							}
							Ok(Outcome::Cleaned) => {}
							Ok(Outcome::Prepared(helper)) => {
								self.helper = Some(helper);
								self.armed = true;
								self.close_requested = true;
								self.status =
									"Update ready. Close Serein to install and restart.".into();
							}
							Err(error) => {
								self.auto_download = false;
								self.status = error;
								self.next_check = Instant::now() + CHECK_INTERVAL;
							}
						}
					}
				}
				Err(mpsc::TryRecvError::Disconnected) => {
					self.job = None;
					self.status = "The update worker stopped. Please try again.".into();
				}
				Err(mpsc::TryRecvError::Empty) => {}
			}
		}
		if self.job.is_none() {
			if let Some(stage) = self.cleanup_pending.take() {
				let helper = self.helper.take();
				self.start(runtime, ctx, 0, move |_, _| async move {
					tokio::task::spawn_blocking(move || {
						if let Some(helper) = helper {
							helper.stop();
						}
						install::cleanup(&stage.directory);
					})
					.await
					.map_err(|_| "Could not clear old update storage.".to_owned())?;
					Ok(Outcome::Cleaned)
				});
			} else if restart && self.staged.is_some() && !self.armed {
				let stage = self.staged.as_ref().expect("checked stage");
				let directory = stage.directory.clone();
				let installation = stage.installation.clone();
				let version = self.package.as_ref().map(|p| p.version.clone());
				let old_helper = self.helper.take();
				self.start(runtime, ctx, 0, move |cancel, _| async move {
					let helper = tokio::task::spawn_blocking(move || {
						if let Some(helper) = old_helper {
							helper.stop();
						}
						if cancel.load(Ordering::Relaxed) {
							return Err("Update cancelled.".into());
						}
						install::prepare_restart(&directory, &installation, version.as_deref())
					})
					.await
					.map_err(|_| "Could not prepare the update restart.".to_owned())??;
					Ok(Outcome::Prepared(helper))
				});
				self.status = "Preparing to restart…".into();
			} else if supported
				&& (download || (view.auto_update && self.auto_download))
				&& self.package.is_some()
				&& self.staged.is_none()
			{
				let package = self.package.as_ref().expect("checked package").clone();
				let total = package
					.archive
					.as_ref()
					.expect("a supported platform's checked package has a downloadable archive")
					.size;
				self.auto_download = false;
				self.status = format!("Downloading Serein {}…", package.version);
				self.start(runtime, ctx, total, move |cancel, progress| {
					download_package(package, cancel, progress)
				});
			} else if self.staged.is_none() && (check || Instant::now() >= self.next_check) {
				if check
					&& self
						.last_check
						.is_some_and(|last| last.elapsed() < Duration::from_secs(60))
				{
					self.status = "Please wait one minute between update checks.".into();
				} else {
					self.last_check = Some(Instant::now());
					self.next_check = Instant::now() + CHECK_INTERVAL;
					let nightly = view.nightly;
					self.start(runtime, ctx, 0, move |cancel, _| async move {
						check_release(nightly, cancel).await.map(Outcome::Checked)
					});
					self.status = "Checking for updates…".into();
				}
			}
		}
		view.busy = self.job.is_some();
		view.progress = self.job.as_ref().filter(|job| job.total > 0).map(|job| {
			(job.progress.load(Ordering::Relaxed) as f64 / job.total as f64).min(1.0) as f32
		});
		view.available = self.package.is_some()
			|| self.staged.is_some()
			|| self.job.as_ref().is_some_and(|job| job.total > 0);
		view.ready = self.staged.is_some() && self.job.is_none();
		view.status.clone_from(&self.status);
		if self.job.is_some() {
			ctx.request_repaint_after(Duration::from_millis(200));
		} else if self.staged.is_none() {
			ctx.request_repaint_after(
				self.next_check
					.saturating_duration_since(Instant::now())
					.max(Duration::from_secs(1)),
			);
		}
		std::mem::take(&mut self.close_requested)
	}
	fn start<F, Fut>(&mut self, runtime: &Runtime, ctx: &egui::Context, total: u64, work: F)
	where
		F: FnOnce(Arc<AtomicBool>, Arc<AtomicU64>) -> Fut + Send + 'static,
		Fut: std::future::Future<Output = Result<Outcome, String>> + Send + 'static,
	{
		let (sender, receiver) = mpsc::sync_channel(1);
		let cancel = Arc::new(AtomicBool::new(false));
		let progress = Arc::new(AtomicU64::new(0));
		let worker_cancel = Arc::clone(&cancel);
		let worker_progress = Arc::clone(&progress);
		let ctx = ctx.clone();
		runtime.spawn(async move {
			let result = work(worker_cancel, worker_progress)
				.await
				.map_err(|error| format!("Update failed: {error}"));
			if let Err(mpsc::SendError(result)) = sender.send(result) {
				tokio::task::spawn_blocking(move || match result {
					Ok(Outcome::Downloaded(stage)) => install::cleanup(&stage.directory),
					Ok(Outcome::Prepared(helper)) => helper.stop(),
					_ => {}
				});
			}
			ctx.request_repaint();
		});
		self.job = Some(Job {
			receiver,
			cancel,
			progress,
			total,
		});
	}
	fn discard_stage(&mut self) {
		if let Some(stage) = self.staged.take() {
			self.cleanup_pending = Some(stage);
		}
	}

	pub fn cancel_restart(&mut self) {
		self.armed = false;
		self.close_requested = false;
		if let Some(job) = &self.job {
			job.cancel.store(true, Ordering::Relaxed);
		}
	}

	/// Called only during the application's approved shutdown, after draft/close gates.
	pub fn finish_restart(&mut self) -> Result<(), String> {
		if self.demo || !self.armed {
			return Ok(());
		}
		if self.staged.is_some() {
			std::fs::write(
				&self
					.helper
					.as_ref()
					.ok_or("No update helper is ready.")?
					.marker,
				[],
			)
			.map_err(|_| "Could not hand off the update to the installer.".to_owned())?;
			self.staged = None; // The helper owns cleanup after this handoff.
			self.helper = None;
		}
		self.armed = false;
		Ok(())
	}
}
impl Drop for Updater {
	fn drop(&mut self) {
		if let Some(job) = &self.job {
			job.cancel.store(true, Ordering::Relaxed);
		}
		let helper = self.helper.take();
		let stage = self.staged.take().or_else(|| self.cleanup_pending.take());
		if helper.is_some() || stage.is_some() {
			std::thread::spawn(move || {
				if let Some(helper) = helper {
					helper.stop();
				}
				if let Some(stage) = stage {
					install::cleanup(&stage.directory);
				}
			});
		}
	}
}

fn client() -> Result<reqwest::Client, String> {
	reqwest::Client::builder()
		.https_only(true)
		.no_proxy()
		.user_agent(concat!("Serein/", env!("CARGO_PKG_VERSION")))
		.connect_timeout(Duration::from_secs(10))
		.read_timeout(Duration::from_secs(30))
		.timeout(Duration::from_secs(600))
		.redirect(reqwest::redirect::Policy::custom(|attempt| {
			if attempt.previous().len() < 5 && trusted_url(attempt.url()) {
				attempt.follow()
			} else {
				attempt.error("Untrusted update redirect")
			}
		}))
		.build()
		.map_err(|_| "Could not initialize secure update transport.".into())
}
fn trusted_url(url: &url::Url) -> bool {
	url.scheme() == "https"
		&& url.port_or_known_default() == Some(443)
		&& url.username().is_empty()
		&& url.password().is_none()
		&& url.fragment().is_none()
		&& matches!(
			url.host_str(),
			Some(
				"api.github.com"
					| "github.com" | "release-assets.githubusercontent.com"
					| "objects.githubusercontent.com"
			)
		)
}
async fn response(client: &reqwest::Client, url: &str) -> Result<reqwest::Response, String> {
	if !url::Url::parse(url).is_ok_and(|url| trusted_url(&url)) {
		return Err("The release contains an untrusted download address.".into());
	}
	let response = client
		.get(url)
		.header(reqwest::header::ACCEPT, "application/vnd.github+json")
		.send()
		.await
		.map_err(|_| "Could not reach GitHub. Check your connection and try again.".to_owned())?;
	match response.status().as_u16() {
		200 => Ok(response),
		403 | 429 => Err("GitHub's update limit was reached. Try again later.".into()),
		404 => Err("No published release is available on this channel yet.".into()),
		_ => Err("GitHub could not provide the update. Try again later.".into()),
	}
}
async fn bounded_body(
	client: &reqwest::Client,
	url: &str,
	limit: usize,
	cancel: &AtomicBool,
) -> Result<Vec<u8>, String> {
	let mut response = response(client, url).await?;
	if response
		.content_length()
		.is_some_and(|length| length > limit as u64)
	{
		return Err("Release metadata exceeds the size limit.".into());
	}
	let mut body = Vec::new();
	while let Some(chunk) = response
		.chunk()
		.await
		.map_err(|_| "The update response was interrupted.".to_owned())?
	{
		if cancel.load(Ordering::Relaxed) {
			return Err("Update cancelled.".into());
		}
		if chunk.len() > limit.saturating_sub(body.len()) {
			return Err("Release metadata exceeds the size limit.".into());
		}
		body.extend_from_slice(&chunk);
	}
	Ok(body)
}
fn release_version(tag: &str) -> Option<semver::Version> {
	if tag.len() > 96
		|| !tag
			.bytes()
			.all(|b| b.is_ascii_alphanumeric() || b".-+".contains(&b))
	{
		return None;
	}
	semver::Version::parse(tag.strip_prefix('v')?).ok()
}
fn asset_name(tag: &str) -> Option<String> {
	if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
		return install::appimage_session().then(|| format!("serein-{tag}-Linux-X64.AppImage"));
	}
	let os = if cfg!(target_os = "macos") {
		"macOS"
	} else if cfg!(windows) {
		"Windows"
	} else {
		return None;
	};
	let arch = if cfg!(target_arch = "aarch64") {
		"ARM64"
	} else if cfg!(target_arch = "x86_64") {
		"X64"
	} else {
		return None;
	};
	Some(format!("serein-{tag}-{os}-{arch}.zip"))
}
fn select_release(
	releases: Vec<Release>,
	nightly: bool,
	current: &semver::Version,
) -> Result<Option<Package>, String> {
	if releases.len() > 100 {
		return Err("The release list exceeds its limit.".into());
	}
	let candidate = releases
		.into_iter()
		.filter_map(|release| {
			let version = release_version(&release.tag_name)?;
			let is_nightly = version.pre.as_str().starts_with("nightly.");
			(!release.draft
				&& ((nightly && release.prerelease && is_nightly)
					|| (!nightly && !release.prerelease && version.pre.is_empty())))
			.then_some((version, release))
		})
		.max_by(|a, b| a.0.cmp(&b.0));
	let Some((version, release)) = candidate else {
		return Err("No published release is available on this channel yet.".into());
	};
	// Channel changes never silently downgrade an installed build.
	if version <= *current {
		return Ok(None);
	}
	if release.assets.len() > 32 {
		return Err("The release contains too many assets.".into());
	}
	// No in-app installer for this installation: still report the newer version so the
	// UI can show it, but there is no asset to look up or download.
	let Some(wanted) = asset_name(&release.tag_name) else {
		return Ok(Some(Package {
			version: version.to_string(),
			archive: None,
			checksums: None,
		}));
	};
	let find = |name: &str| -> Result<Asset, String> {
		let mut found = release.assets.iter().filter(|asset| asset.name == name);
		let asset = found
			.next()
			.ok_or_else(|| format!("This release has no {name} package. Try a later release."))?;
		if found.next().is_some()
			|| asset.size == 0
			|| asset.size > MAX_DOWNLOAD
			|| asset.browser_download_url.len() > 4096
		{
			return Err("The release asset metadata is invalid.".into());
		}
		let expected = format!(
			"https://github.com/ViceVerse-cz/Serein/releases/download/{}/{name}",
			release.tag_name
		);
		if asset.browser_download_url != expected {
			return Err("The asset is not from the Serein release repository.".into());
		}
		Ok(asset.clone())
	};
	let archive = find(&wanted)?;
	let checksums = find("SHA256SUMS.txt")?;
	if checksums.size > 64 * 1024 {
		return Err("The checksum list exceeds its size limit.".into());
	}
	Ok(Some(Package {
		version: version.to_string(),
		archive: Some(archive),
		checksums: Some(checksums),
	}))
}
async fn check_release(nightly: bool, cancel: Arc<AtomicBool>) -> Result<Option<Package>, String> {
	let client = client()?;
	let endpoint = if nightly {
		format!("{RELEASES}?per_page=100")
	} else {
		format!("{RELEASES}/latest")
	};
	let body = bounded_body(&client, &endpoint, MAX_METADATA, &cancel).await?;
	let releases = if nightly {
		serde_json::from_slice::<Vec<Release>>(&body)
	} else {
		serde_json::from_slice::<Release>(&body).map(|r| vec![r])
	}
	.map_err(|_| "GitHub returned invalid release metadata.".to_owned())?;
	let current = semver::Version::parse(env!("CARGO_PKG_VERSION"))
		.map_err(|_| "This build has an invalid version.".to_owned())?;
	select_release(releases, nightly, &current)
}
fn checksum(body: &[u8], name: &str) -> Result<[u8; 32], String> {
	let body = std::str::from_utf8(body).map_err(|_| "The checksum file is invalid.".to_owned())?;
	let mut result = None;
	for line in body.lines() {
		let mut fields = line.split_whitespace();
		let (Some(hash), Some(file)) = (fields.next(), fields.next()) else {
			continue;
		};
		if file
			.trim_start_matches('*')
			.strip_prefix("./")
			.unwrap_or(file.trim_start_matches('*'))
			!= name
		{
			continue;
		}
		if result.is_some()
			|| fields.next().is_some()
			|| hash.len() != 64
			|| !hash.bytes().all(|b| b.is_ascii_hexdigit())
		{
			return Err("The package checksum is invalid or duplicated.".into());
		}
		let mut bytes = [0; 32];
		for (i, byte) in bytes.iter_mut().enumerate() {
			*byte = u8::from_str_radix(&hash[i * 2..i * 2 + 2], 16)
				.map_err(|_| "Invalid checksum.".to_owned())?;
		}
		result = Some(bytes);
	}
	result.ok_or_else(|| "The release has no checksum for this package.".into())
}
async fn download_package(
	package: Package,
	cancel: Arc<AtomicBool>,
	progress: Arc<AtomicU64>,
) -> Result<Outcome, String> {
	// Only reachable on a platform with an in-app installer; `select_release` only omits
	// these when there is nothing to download, and that path never reaches this function.
	let archive = package
		.archive
		.ok_or("This platform cannot install updates in-app.")?;
	let checksums = package
		.checksums
		.ok_or("This platform cannot install updates in-app.")?;
	let client = client()?;
	let expected = checksum(
		&bounded_body(&client, &checksums.browser_download_url, 64 * 1024, &cancel).await?,
		&archive.name,
	)?;
	let stage = tokio::task::spawn_blocking(install::create_stage)
		.await
		.map_err(|_| "Could not prepare update storage.".to_owned())??;
	let directory = stage.directory.clone();
	let result = async {
		use tokio::io::AsyncWriteExt;
		let mut response = response(&client, &archive.browser_download_url).await?;
		if response
			.content_length()
			.is_some_and(|size| size != archive.size)
		{
			return Err("The package size does not match its release metadata.".into());
		}
		let mut file = tokio::fs::OpenOptions::new()
			.write(true)
			.create_new(true)
			.open(directory.join("package.zip"))
			.await
			.map_err(|_| "Could not create the update download.".to_owned())?;
		let mut hasher = Sha256::new();
		let mut received = 0_u64;
		while let Some(chunk) = response
			.chunk()
			.await
			.map_err(|_| "The update download was interrupted.".to_owned())?
		{
			if cancel.load(Ordering::Relaxed) {
				return Err("Update cancelled.".into());
			}
			received = received
				.checked_add(chunk.len() as u64)
				.ok_or("Update size overflow.")?;
			if received > archive.size || received > MAX_DOWNLOAD {
				return Err("The downloaded package exceeds its size limit.".into());
			}
			file.write_all(&chunk).await.map_err(|_| {
				"Could not write the update. Check available disk space.".to_owned()
			})?;
			hasher.update(&chunk);
			progress.store(received, Ordering::Relaxed);
		}
		if received != archive.size || hasher.finalize().as_slice() != expected {
			return Err(
				"The update checksum or length did not match. Nothing was installed.".into(),
			);
		}
		file.sync_all()
			.await
			.map_err(|_| "Could not save the update package.".to_owned())?;
		drop(file);
		let path = directory.clone();
		let installation = stage.installation.clone();
		tokio::task::spawn_blocking(move || install::unpack(&path, &installation, &cancel))
			.await
			.map_err(|_| "Could not unpack the update.".to_owned())??;
		Ok(())
	}
	.await;
	match result {
		Ok(()) => Ok(Outcome::Downloaded(stage)),
		Err(error) => {
			tokio::task::spawn_blocking(move || install::cleanup(&directory))
				.await
				.ok();
			Err(error)
		}
	}
}

/// Offline checks used by the explicit demo debug path, never by release startup.
#[cfg(feature = "demo")]
pub fn debug_check() -> Result<(), String> {
	let stable = release_version("v1.2.3").ok_or("stable version")?;
	let nightly = release_version("v1.2.3-nightly.9.1").ok_or("nightly version")?;
	if nightly >= stable || release_version("v1.2.3/../../bad").is_some() {
		return Err("Version ordering or tag validation failed.".into());
	}
	for address in [
		"http://github.com/x",
		"https://github.com.evil.test/x",
		"https://user@github.com/x",
	] {
		if trusted_url(&url::Url::parse(address).map_err(|_| "URL parse")?) {
			return Err("Untrusted origin accepted.".into());
		}
	}
	let hash = "00".repeat(32);
	if checksum(format!("{hash}  ./package.zip\n").as_bytes(), "package.zip")? != [0; 32]
		|| checksum(
			format!("{hash}  ./package.zip\n{hash}  ./package.zip\n").as_bytes(),
			"package.zip",
		)
		.is_ok()
	{
		return Err("Checksum validation failed.".into());
	}
	install::debug_check()
}
