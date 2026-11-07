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

#[path = "updater_delta.rs"]
mod delta;
#[path = "updater_install.rs"]
mod install;

const RELEASES: &str = "https://api.github.com/repos/sudodc2/Serein/releases";
const FEED: &str = "https://github.com/sudodc2/Serein/releases.atom";
const REPO_DOWNLOADS: &str = "https://github.com/sudodc2/Serein/releases/download";
const MAX_METADATA: usize = 2 * 1024 * 1024;
const MAX_DOWNLOAD: u64 = 512 * 1024 * 1024;
const CHECK_INTERVAL: Duration = Duration::from_secs(60 * 60);
const LOG_RELEASES: usize = 30;
const LOG_NOTES: usize = 16 * 1024;
const LOG_ITEMS: usize = 60;

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
	#[serde(default)]
	published_at: Option<String>,
	#[serde(default)]
	body: Option<String>,
}
#[derive(Clone)]
struct Package {
	version: String,
	// `None` on an installation with no in-app installer: the version is still reported,
	// but there is nothing here to download.
	archive: Option<Asset>,
	checksums: Option<Asset>,
	zsync: Option<Asset>,
}
struct Staged {
	directory: PathBuf,
	installation: PathBuf,
}
enum Outcome {
	Checked(Option<Package>, Vec<ui::updates::LogEntry>),
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
	log: Option<Vec<ui::updates::LogEntry>>,
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
			log: None,
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
				view.log = demo_log();
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
		let supported = (cfg!(any(target_os = "macos", windows)) && !install::nix_session())
			|| install::appimage_session();
		view.supported = supported;
		view.flatpak = install::flatpak_session();
		view.linux_update_cmd = install::linux_package_manager_update_command().map(str::to_owned);
		if !enabled {
			if let Some(job) = &self.job {
				job.cancel.store(true, Ordering::Relaxed);
			}
			view.busy = self.job.is_some();
			view.available = false;
			view.ready = false;
			view.progress = None;
			view.status = if cfg!(debug_assertions) {
				"Update checks are disabled in debug builds."
			} else {
				"Load update preferences or choose your update settings to enable checking."
			}
			.into();
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
			view.log.clear();
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
							Ok(Outcome::Checked(package, log)) => {
								self.log = Some(log);
								self.status = package.as_ref().map_or_else(
									|| "Serein is up to date on this channel.".into(),
									|p| {
										if install::flatpak_session() {
											format!(
												"Serein {} is available. Update with `flatpak update` or your Software center.",
												p.version,
											)
										} else if install::nix_session() {
											format!(
												"Serein {} is available. Update it through Nix.",
												p.version,
											)
										} else if let Some(cmd) =
											install::linux_package_manager_update_command()
										{
											format!(
												"Serein {} is available. Run `{cmd}` to update.",
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
						check_release(nightly, cancel)
							.await
							.map(|(package, log)| Outcome::Checked(package, log))
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
		if let Some(log) = self.log.take() {
			view.log = log;
		}
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
async fn read_bounded(
	mut response: reqwest::Response,
	limit: usize,
	cancel: &AtomicBool,
) -> Result<Vec<u8>, String> {
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
async fn bounded_body(
	client: &reqwest::Client,
	url: &str,
	limit: usize,
	cancel: &AtomicBool,
) -> Result<Vec<u8>, String> {
	read_bounded(response(client, url).await?, limit, cancel).await
}
/// The public releases atom feed: no API rate limit, but only the most recent entries
/// and no draft/prerelease flags, so channel membership comes from the tag alone.
async fn feed_body(client: &reqwest::Client, cancel: &AtomicBool) -> Result<Vec<u8>, String> {
	let response = client
		.get(FEED)
		.header(reqwest::header::ACCEPT, "application/atom+xml")
		.send()
		.await
		.map_err(|_| "Could not reach GitHub. Check your connection and try again.".to_owned())?;
	match response.status().as_u16() {
		200 => read_bounded(response, MAX_METADATA, cancel).await,
		403 | 429 => Err("GitHub's update limit was reached. Try again later.".into()),
		404 => Err("No published release feed is available.".into()),
		_ => Err("GitHub could not provide the update. Try again later.".into()),
	}
}
/// Asset sizes for the feed path come from HEAD requests on the deterministic
/// release-download URLs, the same addresses the API would hand out.
async fn asset_size(client: &reqwest::Client, name: &str, url: &str) -> Result<u64, String> {
	if !url::Url::parse(url).is_ok_and(|url| trusted_url(&url)) {
		return Err("The release contains an untrusted download address.".into());
	}
	let response =
		client.head(url).send().await.map_err(|_| {
			"Could not reach GitHub. Check your connection and try again.".to_owned()
		})?;
	match response.status().as_u16() {
		200 => response
			.content_length()
			.ok_or_else(|| "GitHub did not report the asset size.".to_owned()),
		403 | 429 => Err("GitHub's update limit was reached. Try again later.".into()),
		404 => Err(format!(
			"This release has no {name} package. Try a later release."
		)),
		_ => Err("GitHub could not provide the update. Try again later.".into()),
	}
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
fn on_channel(release: &Release, version: &semver::Version, nightly: bool) -> bool {
	let is_nightly = version.pre.as_str().starts_with("nightly.");
	!release.draft
		&& ((nightly && release.prerelease && is_nightly)
			|| (!nightly && !release.prerelease && version.pre.is_empty()))
}
/// Releases between the running build and the newest one, notes reduced to plain bullets.
fn release_log(
	releases: &[Release],
	nightly: bool,
	current: &semver::Version,
) -> Vec<ui::updates::LogEntry> {
	let mut entries = releases
		.iter()
		.filter_map(|release| {
			let version = release_version(&release.tag_name)?;
			(version > *current && on_channel(release, &version, nightly))
				.then_some((version, release))
		})
		.collect::<Vec<_>>();
	entries.sort_by(|a, b| b.0.cmp(&a.0));
	entries
		.into_iter()
		.take(LOG_RELEASES)
		.map(|(version, release)| ui::updates::LogEntry {
			version: version.to_string(),
			date: release
				.published_at
				.as_deref()
				.and_then(|date| date.get(..10))
				.filter(|date| date.bytes().all(|b| b.is_ascii_digit() || b == b'-'))
				.unwrap_or_default()
				.to_owned(),
			sections: release_notes(release.body.as_deref().unwrap_or_default()),
		})
		.collect()
}
/// Release-please markdown to `(heading, bullets)`: links keep their text, commit hashes go.
fn release_notes(body: &str) -> Vec<(String, Vec<String>)> {
	let body = body
		.get(..body.floor_char_boundary(LOG_NOTES))
		.unwrap_or(body);
	let mut sections: Vec<(String, Vec<String>)> = Vec::new();
	let mut items = 0;
	for line in body.lines().map(str::trim) {
		if let Some(heading) = line.strip_prefix("### ") {
			sections.push((plain_markdown(heading), Vec::new()));
		} else if let Some(item) = line.strip_prefix("* ").or_else(|| line.strip_prefix("- ")) {
			let item = plain_markdown(item);
			if item.is_empty() || items == LOG_ITEMS {
				continue;
			}
			items += 1;
			if sections.is_empty() {
				sections.push((String::new(), Vec::new()));
			}
			sections.last_mut().expect("pushed").1.push(item);
		}
	}
	sections.retain(|(_, items)| !items.is_empty());
	sections
}
fn plain_markdown(text: &str) -> String {
	let mut out = String::with_capacity(text.len());
	let mut rest = text;
	while let Some(open) = rest.find('[') {
		let Some(mid) = rest[open..].find("](").map(|i| open + i) else {
			break;
		};
		let Some(close) = rest[mid..].find(')').map(|i| mid + i) else {
			break;
		};
		out.push_str(&rest[..open]);
		out.push_str(&rest[open + 1..mid]);
		rest = &rest[close + 1..];
	}
	out.push_str(rest);
	let mut out = out.replace("**", "").replace('`', "");
	// Release-please closes every bullet with its commit, ` (abc1234)` once unlinked.
	if out.ends_with(')')
		&& let Some(start) = out.rfind(" (")
		&& out[start + 2..out.len() - 1]
			.bytes()
			.all(|b| b.is_ascii_hexdigit())
	{
		out.truncate(start);
	}
	out.trim().to_owned()
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
			on_channel(&release, &version, nightly).then_some((version, release))
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
			zsync: None,
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
			"https://github.com/sudodc2/Serein/releases/download/{}/{name}",
			release.tag_name
		);
		if asset.browser_download_url != expected {
			return Err("The asset is not from the Serein release repository.".into());
		}
		Ok(asset.clone())
	};
	let archive = find(&wanted)?;
	let checksums = find("SHA256SUMS.txt")?;
	let zsync = wanted
		.ends_with(".AppImage")
		.then(|| find(&format!("{wanted}.zsync")).ok())
		.flatten()
		.filter(|asset| asset.size <= delta::MAX_CONTROL as u64);
	if checksums.size > 64 * 1024 {
		return Err("The checksum list exceeds its size limit.".into());
	}
	Ok(Some(Package {
		version: version.to_string(),
		archive: Some(archive),
		checksums: Some(checksums),
		zsync,
	}))
}
/// One entry of the releases atom feed.
struct FeedRelease {
	tag: String,
	version: semver::Version,
	date: String,
	notes: String,
}
fn feed_on_channel(version: &semver::Version, nightly: bool) -> bool {
	let is_nightly = version.pre.as_str().starts_with("nightly.");
	(nightly && is_nightly) || (!nightly && version.pre.is_empty())
}
fn xml_entity(entity: &str) -> Option<char> {
	match entity {
		"&lt;" => Some('<'),
		"&gt;" => Some('>'),
		"&amp;" => Some('&'),
		"&quot;" => Some('"'),
		"&apos;" => Some('\''),
		_ => {
			let (digits, radix) = entity
				.strip_prefix("&#x")
				.or_else(|| entity.strip_prefix("&#X"))
				.map(|d| (d, 16))
				.or_else(|| entity.strip_prefix("&#").map(|d| (d, 10)))?;
			u32::from_str_radix(digits.trim_end_matches(';'), radix)
				.ok()
				.and_then(char::from_u32)
		}
	}
}
fn xml_unescape(text: &str) -> String {
	let mut out = String::with_capacity(text.len());
	let mut rest = text;
	while let Some(amp) = rest.find('&') {
		out.push_str(&rest[..amp]);
		rest = &rest[amp..];
		let Some(semi) = rest.find(';') else { break };
		let entity = &rest[..=semi];
		match xml_entity(entity) {
			Some(c) => out.push(c),
			None => out.push_str(entity),
		}
		rest = &rest[semi + 1..];
	}
	out.push_str(rest);
	out
}
fn strip_tags(text: &str) -> String {
	let mut out = String::with_capacity(text.len());
	let mut rest = text;
	while let Some(open) = rest.find('<') {
		out.push_str(&rest[..open]);
		match rest[open..].find('>') {
			Some(close) => rest = &rest[open + close + 1..],
			None => return out,
		}
	}
	out.push_str(rest);
	out.trim().to_owned()
}
/// The feed escapes the rendered release body as HTML inside `<content>`; reshape it to
/// the `### heading` / `* bullet` text that `release_notes` already parses.
fn feed_notes(escaped_html: &str) -> String {
	let html = xml_unescape(escaped_html);
	let mut out = String::new();
	let mut rest = html.as_str();
	loop {
		let Some(open) = rest.find('<') else {
			out.push_str(rest);
			break;
		};
		out.push_str(&rest[..open]);
		let after = &rest[open + 1..];
		let Some(close) = after.find('>') else {
			out.push_str(&rest[open..]);
			break;
		};
		let raw = after[..close].trim();
		let name = raw
			.split(|c: char| c.is_ascii_whitespace())
			.next()
			.unwrap_or_default();
		rest = &after[close + 1..];
		let marker = match name {
			"h3" => "### ",
			"li" => "* ",
			_ => continue,
		};
		let end_tag = if name == "h3" { "</h3>" } else { "</li>" };
		if let Some(end) = rest.find(end_tag) {
			out.push('\n');
			out.push_str(marker);
			out.push_str(&strip_tags(&rest[..end]));
			out.push('\n');
			rest = &rest[end + end_tag.len()..];
		}
	}
	out
}
fn feed_entries(xml: &str) -> Vec<FeedRelease> {
	xml.split("<entry>")
		.skip(1)
		.filter_map(|chunk| {
			let chunk = chunk.split("</entry>").next()?;
			let href_start = chunk.find("href=\"")? + 6;
			let href_end = chunk[href_start..].find('"')? + href_start;
			let href = &chunk[href_start..href_end];
			let tag = href.split("/releases/tag/").nth(1)?.to_owned();
			let version = release_version(&tag)?;
			let date = chunk
				.split("<updated>")
				.nth(1)?
				.split("</updated>")
				.next()
				.and_then(|date| date.get(..10))
				.filter(|date| date.bytes().all(|b| b.is_ascii_digit() || b == b'-'))
				.unwrap_or_default()
				.to_owned();
			let notes = chunk
				.find("<content")
				.and_then(|start| {
					let open = chunk[start..].find('>')? + start + 1;
					let inner = &chunk[open..];
					let end = inner.find("</content>")?;
					Some(feed_notes(&inner[..end]))
				})
				.unwrap_or_default();
			Some(FeedRelease {
				tag,
				version,
				date,
				notes,
			})
		})
		.collect()
}
fn feed_log(
	entries: &[FeedRelease],
	nightly: bool,
	current: &semver::Version,
) -> Vec<ui::updates::LogEntry> {
	let mut entries = entries
		.iter()
		.filter(|entry| entry.version > *current && feed_on_channel(&entry.version, nightly))
		.collect::<Vec<_>>();
	entries.sort_by(|a, b| b.version.cmp(&a.version));
	entries
		.into_iter()
		.take(LOG_RELEASES)
		.map(|entry| ui::updates::LogEntry {
			version: entry.version.to_string(),
			date: entry.date.clone(),
			sections: release_notes(&entry.notes),
		})
		.collect()
}
async fn feed_package(client: &reqwest::Client, entry: &FeedRelease) -> Result<Package, String> {
	// No in-app installer for this installation: still report the newer version.
	let Some(wanted) = asset_name(&entry.tag) else {
		return Ok(Package {
			version: entry.version.to_string(),
			archive: None,
			checksums: None,
			zsync: None,
		});
	};
	let base = format!("{REPO_DOWNLOADS}/{}", entry.tag);
	let archive_url = format!("{base}/{wanted}");
	let archive_size = asset_size(client, &wanted, &archive_url).await?;
	if archive_size == 0 || archive_size > MAX_DOWNLOAD {
		return Err("The release asset metadata is invalid.".into());
	}
	let checksums_url = format!("{base}/SHA256SUMS.txt");
	let checksums_size = asset_size(client, "SHA256SUMS.txt", &checksums_url).await?;
	if checksums_size > 64 * 1024 {
		return Err("The checksum list exceeds its size limit.".into());
	}
	let zsync = if wanted.ends_with(".AppImage") {
		let url = format!("{archive_url}.zsync");
		let name = format!("{wanted}.zsync");
		asset_size(client, &name, &url)
			.await
			.ok()
			.filter(|size| *size <= delta::MAX_CONTROL as u64)
			.map(|size| Asset {
				name,
				browser_download_url: url,
				size,
			})
	} else {
		None
	};
	Ok(Package {
		version: entry.version.to_string(),
		archive: Some(Asset {
			name: wanted,
			browser_download_url: archive_url,
			size: archive_size,
		}),
		checksums: Some(Asset {
			name: "SHA256SUMS.txt".into(),
			browser_download_url: checksums_url,
			size: checksums_size,
		}),
		zsync,
	})
}
async fn check_feed(
	client: &reqwest::Client,
	nightly: bool,
	cancel: &AtomicBool,
) -> Result<(Option<Package>, Vec<ui::updates::LogEntry>), String> {
	let body = feed_body(client, cancel).await?;
	let xml = std::str::from_utf8(&body)
		.map_err(|_| "GitHub returned an invalid release feed.".to_owned())?;
	let entries = feed_entries(xml);
	if entries.is_empty() {
		return Err("GitHub returned no releases in the feed.".into());
	}
	let current = semver::Version::parse(env!("CARGO_PKG_VERSION"))
		.map_err(|_| "This build has an invalid version.".to_owned())?;
	let log = feed_log(&entries, nightly, &current);
	let candidate = entries
		.iter()
		.filter(|entry| feed_on_channel(&entry.version, nightly))
		.max_by(|a, b| a.version.cmp(&b.version));
	let Some(entry) = candidate else {
		return Err("No published release is available on this channel yet.".into());
	};
	// Channel changes never silently downgrade an installed build.
	if entry.version <= current {
		return Ok((None, log));
	}
	feed_package(client, entry)
		.await
		.map(|package| (Some(package), log))
}
async fn api_check(
	client: &reqwest::Client,
	nightly: bool,
	cancel: Arc<AtomicBool>,
) -> Result<(Option<Package>, Vec<ui::updates::LogEntry>), String> {
	let endpoint = if nightly {
		format!("{RELEASES}?per_page=100")
	} else {
		format!("{RELEASES}/latest")
	};
	let body = bounded_body(client, &endpoint, MAX_METADATA, &cancel).await?;
	let releases = if nightly {
		serde_json::from_slice::<Vec<Release>>(&body)
	} else {
		serde_json::from_slice::<Release>(&body).map(|r| vec![r])
	}
	.map_err(|_| "GitHub returned invalid release metadata.".to_owned())?;
	let current = semver::Version::parse(env!("CARGO_PKG_VERSION"))
		.map_err(|_| "This build has an invalid version.".to_owned())?;
	let log = release_log(&releases, nightly, &current);
	select_release(releases, nightly, &current).map(|package| (package, log))
}
/// The atom feed has no API rate limit, so it is the primary source. The REST API stays
/// as fallback for feed shapes it cannot express; a feed that parsed but found nothing
/// newer is a definitive answer, not an error.
async fn check_release(
	nightly: bool,
	cancel: Arc<AtomicBool>,
) -> Result<(Option<Package>, Vec<ui::updates::LogEntry>), String> {
	let client = client()?;
	match check_feed(&client, nightly, &cancel).await {
		Ok(result) => Ok(result),
		Err(feed_error) => api_check(&client, nightly, cancel)
			.await
			.map_err(|api_error| format!("{feed_error} GitHub API fallback failed: {api_error}")),
	}
}
fn demo_log() -> Vec<ui::updates::LogEntry> {
	[
		(
			"99.0.0",
			"2026-09-30",
			"Message search filters",
			"Fixed voice reconnects after sleep",
		),
		(
			"98.2.0",
			"2026-09-20",
			"Forum gallery view",
			"Reply previews keep their attachment icon",
		),
	]
	.into_iter()
	.map(|(version, date, feature, fix)| ui::updates::LogEntry {
		version: version.into(),
		date: date.into(),
		sections: vec![
			(
				"Features".into(),
				vec![format!("Synthetic preview: {feature}")],
			),
			(
				"Bug Fixes".into(),
				vec![format!("Synthetic preview: {fix}")],
			),
		],
	})
	.collect()
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
	let checksum_body =
		bounded_body(&client, &checksums.browser_download_url, 64 * 1024, &cancel).await?;
	let expected = checksum(&checksum_body, &archive.name)?;
	let stage = tokio::task::spawn_blocking(install::create_stage)
		.await
		.map_err(|_| "Could not prepare update storage.".to_owned())??;
	let directory = stage.directory.clone();
	let result = async {
		let reused = if let Some(control) = package.zsync.filter(|_| install::appimage_session()) {
			delta::download(
				&client,
				&archive,
				&control,
				&checksum_body,
				&stage,
				&cancel,
				&progress,
			)
			.await
			.is_ok()
		} else {
			false
		};
		if cancel.load(Ordering::Relaxed) {
			return Err("Update cancelled.".into());
		}
		if !reused {
			let partial = directory.join("package.zip");
			match tokio::fs::remove_file(&partial).await {
				Ok(()) => {}
				Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
				Err(_) => return Err("Could not discard the partial update.".into()),
			}
			progress.store(0, Ordering::Relaxed);
			download_full(&client, &archive, &directory, &expected, &cancel, &progress).await?;
		}
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

async fn download_full(
	client: &reqwest::Client,
	archive: &Asset,
	directory: &std::path::Path,
	expected: &[u8; 32],
	cancel: &AtomicBool,
	progress: &AtomicU64,
) -> Result<(), String> {
	use tokio::io::AsyncWriteExt;
	let mut response = response(client, &archive.browser_download_url).await?;
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
		file.write_all(&chunk)
			.await
			.map_err(|_| "Could not write the update. Check available disk space.".to_owned())?;
		hasher.update(&chunk);
		progress.store(received, Ordering::Relaxed);
	}
	if received != archive.size || hasher.finalize().as_slice() != expected.as_slice() {
		return Err("The update checksum or length did not match. Nothing was installed.".into());
	}
	file.sync_all()
		.await
		.map_err(|_| "Could not save the update package.".to_owned())?;
	drop(file);
	Ok(())
}

/// Offline checks used by the explicit demo debug path, never by release startup.
#[cfg(feature = "demo")]
pub fn debug_check() -> Result<(), String> {
	delta::debug_check()?;
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
	let notes = release_notes(
		"## [1.0.0](https://x) (2026-09-28)\n\n### Features\n\n* **voice:** add preview by [@a](https://x) in [#457](https://x) ([f15eac0](https://x))\n",
	);
	if notes
		!= [(
			"Features".to_owned(),
			vec!["voice: add preview by @a in #457".to_owned()],
		)] {
		return Err(format!("Release notes parsed as {notes:?}."));
	}
	let feed = concat!(
		"<feed><entry>",
		"<link rel=\"alternate\" type=\"text/html\" href=\"https://github.com/sudodc2/Serein/releases/tag/v9.9.9\"/>",
		"<updated>2026-10-01T12:00:00Z</updated>",
		"<content type=\"html\">&lt;h3&gt;Features&lt;/h3&gt;&lt;ul&gt;&lt;li&gt;Add &lt;a href=&quot;https://x&quot;&gt;thing&lt;/a&gt; (abc1234)&lt;/li&gt;&lt;/ul&gt;</content>",
		"</entry></feed>"
	);
	let entries = feed_entries(feed);
	if entries.len() != 1
		|| entries[0].version != semver::Version::new(9, 9, 9)
		|| entries[0].date != "2026-10-01"
	{
		return Err("Release feed entries did not parse.".into());
	}
	let log = feed_log(&entries, false, &semver::Version::new(1, 0, 0));
	if log.len() != 1 || log[0].sections != [("Features".to_owned(), vec!["Add thing".to_owned()])]
	{
		return Err(format!("Feed notes parsed as {log:?}."));
	}
	if feed_on_channel(&entries[0].version, true) || !feed_on_channel(&entries[0].version, false) {
		return Err("Feed channel filter failed.".into());
	}
	install::debug_check()
}
