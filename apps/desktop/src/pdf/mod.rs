//! One lazy latest-request worker; documents and page pixels are RAM-only.
mod protocol;
mod renderer;
use std::{
	process::{Command, Stdio},
	sync::{
		Arc, Mutex,
		atomic::{AtomicBool, Ordering},
	},
	time::{Duration, Instant},
};
use ui::{MAX_PDF_BYTES, PdfCommand, PdfError, PdfRequest, PdfUi};
const HELPER_ARG: &str = "--serein-pdf-helper";
const WALL_TIME: Duration = Duration::from_secs(10);
const POLL: Duration = Duration::from_millis(20);
/// An unverified reap permanently retires this worker; never overlap helpers.
#[derive(Debug, PartialEq, Eq)]
enum HelperFailure {
	Preview(PdfError),
	Unreaped,
}
impl From<PdfError> for HelperFailure {
	fn from(error: PdfError) -> Self {
		Self::Preview(error)
	}
}
impl HelperFailure {
	fn public(&self) -> PdfError {
		match self {
			Self::Preview(error) => *error,
			Self::Unreaped => PdfError::Unavailable,
		}
	}
}
struct Rendered {
	pages: u32,
	page: u32,
	image: Arc<eframe::egui::ColorImage>,
}
struct Session {
	cancelled: AtomicBool,
	result: Mutex<Option<Result<Rendered, PdfError>>>,
}
#[derive(Clone)]
struct Request {
	session: Arc<Session>,
	job: PdfRequest,
	target: [u16; 2],
	demo: bool,
}
#[derive(Default)]
pub struct Preview {
	requests: Option<tokio::sync::watch::Sender<Option<Request>>>,
	active: Option<Request>,
}
impl Preview {
	pub fn stop(&mut self) {
		if let Some(r) = self.active.take() {
			r.session.cancelled.store(true, Ordering::Release);
			if let Some(sender) = &self.requests {
				sender.send_replace(None);
			}
		}
	}
	pub fn poll(&self, view: &mut PdfUi, ctx: &eframe::egui::Context) {
		let Some(r) = &self.active else {
			return;
		};
		let result = r.session.result.try_lock().ok().and_then(|mut s| s.take());
		if let Some(result) = result {
			match result {
				Ok(raster) => view.accept(
					ctx,
					r.job.document,
					r.job.request,
					raster.pages,
					raster.page,
					raster.image,
				),
				Err(e) => view.fail(r.job.document, r.job.request, e),
			}
		}
	}
	pub fn command(
		&mut self,
		command: PdfCommand,
		view: &mut PdfUi,
		runtime: &tokio::runtime::Handle,
		ctx: &eframe::egui::Context,
		demo: bool,
	) {
		let PdfCommand::Render(job) = command else {
			self.stop();
			return;
		};
		let job = *job;
		if let Some(r) = self.active.take() {
			r.session.cancelled.store(true, Ordering::Release);
		}
		if self.requests.is_none() {
			let (sender, receiver) = tokio::sync::watch::channel::<Option<Request>>(None);
			let runtime = runtime.clone();
			let fetch_runtime = runtime.clone();
			let ctx = ctx.clone();
			if std::thread::Builder::new()
				.name("serein-pdf-preview".into())
				.spawn(move || {
					work_requests(
						receiver,
						&runtime,
						&ctx,
						|r| fetch_runtime.block_on(fetch(r)),
						|bytes, page, target, cancel| {
							run_helper(
								bytes,
								page,
								target,
								cancel,
								&std::env::current_exe().map_err(|_| PdfError::Unavailable)?,
								WALL_TIME,
							)
						},
					);
				})
				.is_err()
			{
				view.fail(job.document, job.request, PdfError::Unavailable);
				return;
			}
			self.requests = Some(sender);
		}
		if self.requests.as_ref().is_none_or(|s| s.is_closed()) {
			view.fail(job.document, job.request, PdfError::Unavailable);
			return;
		}
		let r = Request {
			session: Arc::new(Session {
				cancelled: AtomicBool::new(false),
				result: Mutex::new(None),
			}),
			job,
			target: ui::pdf_render_target(ctx),
			demo,
		};
		self.requests
			.as_ref()
			.unwrap()
			.send_replace(Some(r.clone()));
		self.active = Some(r);
	}
}
/// One running request plus the latest watch value. Test injection exercises the
/// actual scheduling/cache path without network, UI, or a second parser.
fn work_requests(
	mut receiver: tokio::sync::watch::Receiver<Option<Request>>,
	runtime: &tokio::runtime::Handle,
	ctx: &eframe::egui::Context,
	mut load: impl FnMut(&Request) -> Result<Vec<u8>, PdfError>,
	mut render: impl FnMut(
		Arc<Vec<u8>>,
		u32,
		[u16; 2],
		&AtomicBool,
	) -> Result<protocol::Raster, HelperFailure>,
) {
	let mut cache: Option<(u64, Arc<Vec<u8>>)> = None;
	while runtime.block_on(receiver.changed()).is_ok() {
		let request = receiver.borrow_and_update().clone();
		let Some(r) = request else {
			cache = None;
			continue;
		};
		if cancelled(&r) {
			cache = None;
			continue;
		}
		if cache
			.as_ref()
			.is_some_and(|(doc, _)| *doc != r.job.document)
		{
			cache = None;
		}
		let mut retire = false;
		let result = (|| {
			validate_attachment(&r.job.attachment, r.demo)?;
			if cache.is_none() {
				let data = load(&r)?;
				if cancelled(&r) {
					return Err(PdfError::Unavailable);
				}
				cache = Some((r.job.document, Arc::new(data)));
			}
			let bytes = cache.as_ref().expect("loaded current document").1.clone();
			render(bytes, r.job.page, r.target, &r.session.cancelled).map_err(|failure| {
				retire = failure == HelperFailure::Unreaped;
				failure.public()
			})
		})();
		if retire {
			// Fail the latest slot too, even if this job was superseded.
			if let Some(latest) = receiver.borrow().as_ref()
				&& let Ok(mut slot) = latest.session.result.lock()
			{
				*slot = Some(Err(PdfError::Unavailable));
			}
			ctx.request_repaint();
			break;
		}
		if cancelled(&r) {
			if receiver
				.borrow()
				.as_ref()
				.is_none_or(|next| next.job.document != r.job.document)
			{
				cache = None;
			}
			continue;
		}
		let result = result.map(|raster| Rendered {
			pages: raster.pages,
			page: raster.page,
			image: Arc::new(eframe::egui::ColorImage::from_rgba_premultiplied(
				[raster.width, raster.height],
				&raster.rgba,
			)),
		});
		if let Ok(mut slot) = r.session.result.lock() {
			*slot = Some(result);
		}
		ctx.request_repaint();
	}
}

impl Drop for Preview {
	fn drop(&mut self) {
		self.stop();
	}
}
fn cancelled(r: &Request) -> bool {
	r.session.cancelled.load(Ordering::Acquire)
}
fn validate_attachment(a: &model::Attachment, demo: bool) -> Result<(), PdfError> {
	if !a.is_pdf() || !model::valid_attachments(std::slice::from_ref(a)) {
		return Err(PdfError::Invalid);
	}
	if a.size == 0 || a.size > MAX_PDF_BYTES as u64 {
		return Err(PdfError::Limit);
	}
	if !demo && crate::downloads::original_url(a).is_none() {
		return Err(PdfError::Unavailable);
	}
	Ok(())
}
fn mime(value: Option<&str>) -> bool {
	value.is_none_or(|s| {
		matches!(
			s.split(';')
				.next()
				.unwrap_or(s)
				.trim()
				.to_ascii_lowercase()
				.as_str(),
			"application/pdf" | "application/octet-stream" | "binary/octet-stream"
		)
	})
}
async fn fetch(r: &Request) -> Result<Vec<u8>, PdfError> {
	#[cfg(feature = "demo")]
	if r.demo {
		return Ok(include_bytes!("../../tests/fixtures/pdf-preview.pdf").to_vec());
	}
	if r.demo {
		return Err(PdfError::Unavailable);
	}
	let url = crate::downloads::original_url(&r.job.attachment).ok_or(PdfError::Unavailable)?;
	let client = reqwest::Client::builder()
		.redirect(reqwest::redirect::Policy::none())
		.no_proxy()
		.no_gzip()
		.no_brotli()
		.no_deflate()
		.no_zstd()
		.timeout(WALL_TIME)
		.build()
		.map_err(|_| PdfError::Unavailable)?;
	let work = async {
		let response = client.get(url).send().await.map_err(|e| {
			if e.is_timeout() {
				PdfError::TimedOut
			} else {
				PdfError::Unavailable
			}
		})?;
		read_document(response, r.job.attachment.size).await
	};
	tokio::pin!(work);
	loop {
		tokio::select! {result=&mut work=>return result,_=tokio::time::sleep(POLL)=>{if cancelled(r){return Err(PdfError::Unavailable);}}}
	}
}
/// Validate headers before allocating, then enforce both declared and absolute lengths.
async fn read_document(response: reqwest::Response, declared: u64) -> Result<Vec<u8>, PdfError> {
	if declared == 0 || declared > MAX_PDF_BYTES as u64 {
		return Err(PdfError::Limit);
	}
	if !response.status().is_success() {
		return Err(PdfError::Unavailable);
	}
	if response
		.content_length()
		.is_some_and(|n| n > MAX_PDF_BYTES as u64 || n > declared)
	{
		return Err(PdfError::Limit);
	}
	let content_type = response
		.headers()
		.get(reqwest::header::CONTENT_TYPE)
		.map(|h| h.to_str().map_err(|_| PdfError::Invalid))
		.transpose()?;
	if !mime(content_type)
		|| response
			.headers()
			.contains_key(reqwest::header::CONTENT_ENCODING)
	{
		return Err(PdfError::Invalid);
	}
	use futures_util::StreamExt;
	let mut stream = response.bytes_stream();
	let mut data = Vec::with_capacity(declared as usize);
	while let Some(chunk) = stream.next().await {
		let chunk = chunk.map_err(|_| PdfError::Unavailable)?;
		if data.len().saturating_add(chunk.len()) > MAX_PDF_BYTES
			|| data.len().saturating_add(chunk.len()) > declared as usize
		{
			return Err(PdfError::Limit);
		}
		data.extend_from_slice(&chunk);
	}
	if data.len() != declared as usize {
		return Err(PdfError::Unavailable);
	}
	if !data.starts_with(b"%PDF-") {
		return Err(PdfError::Invalid);
	}
	Ok(data)
}
/// Kill, wait/reap and finish pipe I/O before allowing the next helper. No detached parser.
fn run_helper(
	bytes: Arc<Vec<u8>>,
	page: u32,
	target: [u16; 2],
	cancel: &AtomicBool,
	exe: &std::path::Path,
	timeout: Duration,
) -> Result<protocol::Raster, HelperFailure> {
	if cancel.load(Ordering::Acquire) {
		return Err(PdfError::Unavailable.into());
	}
	let mut command = Command::new(exe);
	command
		.arg(HELPER_ARG)
		.env_clear()
		.stdin(Stdio::piped())
		.stdout(Stdio::piped())
		.stderr(Stdio::null());
	#[cfg(target_os = "windows")]
	{
		use std::os::windows::process::CommandExt;
		command.creation_flags(0x08000000);
		if let Some(root) = std::env::var_os("SYSTEMROOT") {
			command.env("SYSTEMROOT", root);
		}
	}
	let mut child = command.spawn().map_err(|_| PdfError::Unavailable)?;
	let stdin = child.stdin.take().expect("piped stdin");
	let stdout = child.stdout.take().expect("piped stdout");
	let io = match std::thread::Builder::new()
		.name("serein-pdf-pipe".into())
		.spawn(move || {
			let mut stdin = stdin;
			protocol::write_request(&mut stdin, page, target, &bytes)?;
			drop(stdin);
			protocol::read_response(&mut { stdout }, page)
		}) {
		Ok(io) => io,
		Err(_) => {
			reap_helper(&mut child, true)?;
			return Err(PdfError::Unavailable.into());
		}
	};
	let start = Instant::now();
	let mut failure = None;
	let status = loop {
		if cancel.load(Ordering::Acquire) {
			failure = Some(PdfError::Unavailable);
			break None;
		}
		if start.elapsed() >= timeout {
			failure = Some(PdfError::TimedOut);
			break None;
		}
		match platform::pdf::memory_exceeded(child.id()) {
			Ok(true) => {
				failure = Some(PdfError::Limit);
				break None;
			}
			Ok(false) => {}
			Err(_) => {
				if child.try_wait().is_ok_and(|s| s.is_some()) {
					break Some(());
				}
				failure = Some(PdfError::Unavailable);
				break None;
			}
		}
		match child.try_wait() {
			Ok(Some(_)) => break Some(()),
			Ok(None) => std::thread::sleep(POLL),
			Err(_) => {
				failure = Some(PdfError::Renderer);
				break None;
			}
		}
	};
	let reaped = reap_helper(&mut child, status.is_none());
	// Join even on cleanup failure. The real helper has an independent watchdog;
	// neither a detached pipe thread nor a replacement parser may outlive this job.
	let result = io.join().map_err(|_| PdfError::Renderer);
	let reaped = reaped?;
	if let Some(e) = failure {
		return Err(e.into());
	}
	if !reaped.success() {
		return Err(PdfError::Renderer.into());
	}
	Ok(result??)
}
fn reap_helper(
	child: &mut std::process::Child,
	kill: bool,
) -> Result<std::process::ExitStatus, HelperFailure> {
	if kill {
		// Kill can race with normal exit. Only a successful wait verifies cleanup.
		let _ = child.kill();
	}
	loop {
		match child.wait() {
			Ok(status) => return Ok(status),
			Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
			Err(_) => return Err(HelperFailure::Unreaped),
		}
	}
}
pub fn helper_requested() -> bool {
	{
		let mut args = std::env::args_os().skip(1);
		args.next().is_some_and(|arg| arg == HELPER_ARG) && args.next().is_none()
	}
}
pub fn helper_main() {
	// Apply limits before reading attacker-controlled lengths, opening a document or fonts.
	let _watchdog = std::thread::spawn(|| {
		std::thread::sleep(WALL_TIME);
		std::process::exit(2);
	});
	let guard = platform::pdf::constrain_helper();
	let result = guard
		.as_ref()
		.map_err(|_| PdfError::Unavailable)
		.and_then(|_| {
			let (page, target, data) = protocol::read_request(&mut std::io::stdin().lock())?;
			renderer::render(data, page, target)
		});
	let _ = protocol::write_response(&mut std::io::stdout().lock(), &result);
}
#[cfg(test)]
mod tests {
	use super::*;
	use ui::MAX_PDF_PAGES;
	#[test]
	fn response_mime_is_pdf_or_generic_only() {
		for m in [
			None,
			Some("application/pdf"),
			Some("Application/PDF; charset=binary"),
			Some("application/octet-stream"),
		] {
			assert!(mime(m));
		}
		for m in [
			Some("text/html"),
			Some("image/png"),
			Some("application/pdf-evil"),
		] {
			assert!(!mime(m));
		}
	}

	#[test]
	fn metadata_and_cdn_admission_precede_fetch() {
		let state = test_support::pdf_demo_state();
		let mut a = state.timeline.get(model::Id(501)).unwrap().attachments[0].clone();
		assert!(validate_attachment(&a, false).is_ok());
		for url in [
			"http://cdn.discordapp.com/attachments/20/701/test.pdf",
			"https://example.org/test.pdf",
			"https://cdn.discordapp.com/attachments/20/999/test.pdf",
			"https://user:password@cdn.discordapp.com/attachments/20/701/test.pdf",
			"https://cdn.discordapp.com/attachments/20/701/test.pdf?width=200",
		] {
			a.media.url = Some(url.into());
			assert_eq!(
				validate_attachment(&a, false).err(),
				Some(PdfError::Unavailable)
			);
		}
		a.size = MAX_PDF_BYTES as u64 + 1;
		assert_eq!(validate_attachment(&a, true).err(), Some(PdfError::Limit));
		a.size = 0;
		assert_eq!(validate_attachment(&a, true).err(), Some(PdfError::Limit));
		a.size = 10;
		a.filename = "image.png".into();
		a.content_type = Some("image/png".into());
		assert_eq!(validate_attachment(&a, true).err(), Some(PdfError::Invalid));
	}
	#[cfg(unix)]
	#[test]
	fn timeout_and_cancellation_reap_helper_before_returning() {
		use std::os::unix::fs::PermissionsExt;
		let path =
			std::env::temp_dir().join(format!("serein-pdf-helper-test-{}.sh", std::process::id()));
		let marker = path.with_extension("pid");
		let script = format!(
			"#!/bin/sh\necho $$ > '{}'\nexec /bin/sleep 30\n",
			marker.display()
		);
		std::fs::write(&path, script).unwrap();
		std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
		let cancel = AtomicBool::new(false);
		let start = Instant::now();
		assert_eq!(
			run_helper(
				Arc::new(b"%PDF-test".to_vec()),
				0,
				[1536, 940],
				&cancel,
				&path,
				Duration::from_millis(100)
			)
			.err()
			.map(|e| e.public()),
			Some(PdfError::TimedOut)
		);
		assert!(start.elapsed() < Duration::from_secs(3));
		let first = std::fs::read_to_string(&marker).unwrap();
		#[cfg(target_os = "linux")]
		assert!(
			!std::path::Path::new(&format!("/proc/{}", first.trim())).exists(),
			"timed-out helper was reaped"
		);
		#[cfg(not(target_os = "linux"))]
		let _ = first;
		std::fs::remove_file(&marker).unwrap();
		std::thread::scope(|scope| {
			scope.spawn(|| {
				std::thread::sleep(Duration::from_millis(100));
				cancel.store(true, Ordering::Release);
			});
			assert_eq!(
				run_helper(
					Arc::new(b"%PDF-test".to_vec()),
					0,
					[1536, 940],
					&cancel,
					&path,
					Duration::from_secs(3)
				)
				.err()
				.map(|e| e.public()),
				Some(PdfError::Unavailable)
			);
		});
		let second = std::fs::read_to_string(&marker).unwrap();
		#[cfg(target_os = "linux")]
		assert!(
			!std::path::Path::new(&format!("/proc/{}", second.trim())).exists(),
			"cancelled helper was reaped"
		);
		#[cfg(not(target_os = "linux"))]
		let _ = second;
		std::fs::remove_file(marker).unwrap();
		std::fs::remove_file(path).unwrap();
	}
	#[test]
	fn already_cancelled_request_never_spawns_a_helper() {
		assert_eq!(
			run_helper(
				Arc::new(b"%PDF-test".to_vec()),
				0,
				[1536, 940],
				&AtomicBool::new(true),
				std::path::Path::new("missing-executable"),
				WALL_TIME
			)
			.err()
			.map(|e| e.public()),
			Some(PdfError::Unavailable)
		);
	}
	fn request(document: u64, page: u32) -> Request {
		let state = test_support::pdf_demo_state();
		Request {
			session: Arc::new(Session {
				cancelled: AtomicBool::new(false),
				result: Mutex::new(None),
			}),
			job: PdfRequest {
				document,
				request: page as u64 + 1,
				page,
				attachment: state.timeline.get(model::Id(501)).unwrap().attachments[0].clone(),
			},
			target: [1536, 940],
			demo: true,
		}
	}
	fn wait_until(mut ready: impl FnMut() -> bool) {
		let start = Instant::now();
		while !ready() {
			assert!(start.elapsed() < Duration::from_secs(3));
			std::thread::sleep(Duration::from_millis(5));
		}
	}
	#[test]
	fn latest_slot_coalesces_rapid_navigation_and_releases_closed_document_cache() {
		use std::sync::mpsc;
		let runtime = tokio::runtime::Runtime::new().unwrap();
		let (sender, receiver) = tokio::sync::watch::channel(None);
		let (loads, loaded) = mpsc::channel();
		let (renders, rendered) = mpsc::channel();
		let (proceed, gate) = mpsc::sync_channel(1);
		let worker_runtime = runtime.handle().clone();
		let worker = std::thread::spawn(move || {
			work_requests(
				receiver,
				&worker_runtime,
				&eframe::egui::Context::default(),
				|r| {
					loads.send(r.job.document).unwrap();
					Ok(b"%PDF-synthetic".to_vec())
				},
				|bytes, page, target, cancel| {
					assert_eq!(target, [1536, 940]);
					renders.send((page, Arc::downgrade(&bytes))).unwrap();
					gate.recv_timeout(Duration::from_secs(3)).unwrap();
					if cancel.load(Ordering::Acquire) {
						return Err(PdfError::Unavailable.into());
					}
					Ok(protocol::Raster {
						pages: MAX_PDF_PAGES,
						page,
						width: 1,
						height: 1,
						rgba: vec![255; 4],
					})
				},
			);
		});
		let first = request(10, 0);
		sender.send_replace(Some(first.clone()));
		assert_eq!(loaded.recv_timeout(Duration::from_secs(3)).unwrap(), 10);
		let (page, first_bytes) = rendered.recv_timeout(Duration::from_secs(3)).unwrap();
		assert_eq!(page, 0);
		let mut previous = first.clone();
		let mut superseded = Vec::new();
		for page in 1..200 {
			previous.session.cancelled.store(true, Ordering::Release);
			superseded.push(previous);
			previous = request(10, page);
			sender.send_replace(Some(previous.clone()));
		}
		proceed.send(()).unwrap();
		let (page, _) = rendered.recv_timeout(Duration::from_secs(3)).unwrap();
		assert_eq!(page, 199, "intermediate requests are coalesced, not queued");
		assert!(
			loaded.try_recv().is_err(),
			"page changes reuse one encoded document"
		);
		proceed.send(()).unwrap();
		wait_until(|| previous.session.result.lock().unwrap().is_some());
		assert!(
			superseded
				.iter()
				.all(|r| r.session.result.lock().unwrap().is_none()),
			"cancelled requests never publish a raster"
		);
		assert!(
			previous
				.session
				.result
				.lock()
				.unwrap()
				.as_ref()
				.unwrap()
				.is_ok()
		);
		previous.session.cancelled.store(true, Ordering::Release);
		let second = request(20, 0);
		sender.send_replace(Some(second.clone()));
		assert_eq!(loaded.recv_timeout(Duration::from_secs(3)).unwrap(), 20);
		let (_, second_bytes) = rendered.recv_timeout(Duration::from_secs(3)).unwrap();
		assert!(
			first_bytes.upgrade().is_none(),
			"document replacement releases old bytes"
		);
		second.session.cancelled.store(true, Ordering::Release);
		sender.send_replace(None);
		proceed.send(()).unwrap();
		wait_until(|| second_bytes.upgrade().is_none());
		assert!(second.session.result.lock().unwrap().is_none());
		let reopened = request(20, 0);
		sender.send_replace(Some(reopened.clone()));
		assert_eq!(loaded.recv_timeout(Duration::from_secs(3)).unwrap(), 20);
		let (_, reopened_bytes) = rendered.recv_timeout(Duration::from_secs(3)).unwrap();
		proceed.send(()).unwrap();
		wait_until(|| reopened.session.result.lock().unwrap().is_some());
		drop(sender);
		worker.join().unwrap();
		assert!(reopened_bytes.upgrade().is_none());
	}
	#[test]
	fn unverified_helper_cleanup_retires_worker_and_fails_latest_request() {
		use std::sync::mpsc;
		let runtime = tokio::runtime::Runtime::new().unwrap();
		let (sender, receiver) = tokio::sync::watch::channel(None);
		let (started, running) = mpsc::channel();
		let (proceed, gate) = mpsc::sync_channel(1);
		let handle = runtime.handle().clone();
		let worker = std::thread::spawn(move || {
			work_requests(
				receiver,
				&handle,
				&eframe::egui::Context::default(),
				|_| Ok(b"%PDF-synthetic".to_vec()),
				|_, _, _, _| {
					started.send(()).unwrap();
					gate.recv_timeout(Duration::from_secs(3)).unwrap();
					Err(HelperFailure::Unreaped)
				},
			);
		});
		let first = request(10, 0);
		sender.send_replace(Some(first.clone()));
		running.recv_timeout(Duration::from_secs(3)).unwrap();
		first.session.cancelled.store(true, Ordering::Release);
		let latest = request(20, 0);
		sender.send_replace(Some(latest.clone()));
		proceed.send(()).unwrap();
		worker.join().unwrap();
		assert!(
			sender.is_closed(),
			"never start another helper after failed cleanup"
		);
		assert!(matches!(
			latest.session.result.lock().unwrap().as_ref(),
			Some(Err(PdfError::Unavailable))
		));
		assert!(first.session.result.lock().unwrap().is_none());
		assert!(running.try_recv().is_err());
	}
	#[test]
	fn worker_preserves_premultiplied_pixels_and_requested_envelope() {
		let runtime = tokio::runtime::Runtime::new().unwrap();
		let (sender, receiver) = tokio::sync::watch::channel(None);
		let handle = runtime.handle().clone();
		let worker = std::thread::spawn(move || {
			work_requests(
				receiver,
				&handle,
				&eframe::egui::Context::default(),
				|_| Ok(b"%PDF-synthetic".to_vec()),
				|_, page, target, _| {
					assert_eq!(target, [816, 220]);
					Ok(protocol::Raster {
						pages: 2,
						page,
						width: 1,
						height: 1,
						rgba: vec![64, 32, 16, 128],
					})
				},
			);
		});
		let mut request = request(10, 0);
		request.target = [816, 220];
		sender.send_replace(Some(request.clone()));
		wait_until(|| request.session.result.lock().unwrap().is_some());
		let result = request
			.session
			.result
			.lock()
			.unwrap()
			.take()
			.unwrap()
			.unwrap();
		assert_eq!(result.image.pixels[0].to_array(), [64, 32, 16, 128]);
		drop(sender);
		worker.join().unwrap();
	}
	#[test]
	fn cancelled_completed_raster_skips_color_conversion_and_publication() {
		let runtime = tokio::runtime::Runtime::new().unwrap();
		let (sender, receiver) = tokio::sync::watch::channel(None);
		let (finished, completed) = std::sync::mpsc::channel();
		let handle = runtime.handle().clone();
		let worker = std::thread::spawn(move || {
			work_requests(
				receiver,
				&handle,
				&eframe::egui::Context::default(),
				|_| Ok(b"%PDF-synthetic".to_vec()),
				|_, page, _, cancel| {
					cancel.store(true, Ordering::Release);
					finished.send(()).unwrap();
					// Only this injected test uses invalid pixels: attempting
					// conversion after cancellation would panic on the length.
					Ok(protocol::Raster {
						pages: 2,
						page,
						width: 1,
						height: 1,
						rgba: Vec::new(),
					})
				},
			);
		});
		let request = request(10, 0);
		sender.send_replace(Some(request.clone()));
		completed.recv_timeout(Duration::from_secs(3)).unwrap();
		drop(sender);
		worker.join().unwrap();
		assert!(request.session.result.lock().unwrap().is_none());
	}

	#[tokio::test]
	async fn response_headers_stream_lengths_and_magic_are_bounded() {
		async fn response(
			headers: &str,
			body: &[u8],
		) -> (reqwest::Response, std::thread::JoinHandle<()>) {
			use std::io::{Read, Write};
			let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
			let address = listener.local_addr().unwrap();
			let bytes = [
				format!("HTTP/1.1 200 OK\r\nConnection: close\r\n{headers}\r\n").as_bytes(),
				body,
			]
			.concat();
			let server = std::thread::spawn(move || {
				let (mut stream, _) = listener.accept().unwrap();
				let mut request = [0; 4096];
				let _ = stream.read(&mut request);
				// Header rejection may legitimately close before the body is sent.
				let _ = stream.write_all(&bytes);
			});
			(
				reqwest::Client::builder()
					.no_proxy()
					.no_gzip()
					.build()
					.unwrap()
					.get(format!("http://{address}"))
					.send()
					.await
					.unwrap(),
				server,
			)
		}
		async fn read(headers: &str, body: &[u8], declared: u64) -> Result<Vec<u8>, PdfError> {
			let (response, server) = response(headers, body).await;
			let result = read_document(response, declared).await;
			server.join().unwrap();
			result
		}
		let pdf = b"%PDF-test";
		assert_eq!(
			read(
				"Content-Type: application/pdf\r\nContent-Length: 9\r\n",
				pdf,
				9
			)
			.await
			.unwrap(),
			pdf
		);
		assert_eq!(
			read("Content-Type: text/html\r\n", pdf, 9).await.err(),
			Some(PdfError::Invalid)
		);
		assert_eq!(
			read("Content-Encoding: gzip\r\n", pdf, 9).await.err(),
			Some(PdfError::Invalid)
		);
		assert_eq!(
			read("Content-Length: 999999999\r\n", pdf, 9).await.err(),
			Some(PdfError::Limit)
		);
		assert_eq!(read("", pdf, 8).await.err(), Some(PdfError::Limit));
		assert_eq!(
			read("Content-Type: application/pdf\r\n", b"not a pdf", 9)
				.await
				.err(),
			Some(PdfError::Invalid)
		);
		assert_eq!(
			read(
				"Transfer-Encoding: chunked\r\n",
				b"9\r\n%PDF-test\r\n0\r\n\r\n",
				8
			)
			.await
			.err(),
			Some(PdfError::Limit)
		);
		assert_eq!(read("", pdf, 10).await.err(), Some(PdfError::Unavailable));
		assert_eq!(
			read("Content-Length: 10\r\n", pdf, 10).await.err(),
			Some(PdfError::Unavailable)
		);
		assert_eq!(
			read(
				"Transfer-Encoding: chunked\r\n",
				b"9\r\n%PDF-test\r\n0\r\n\r\n",
				10
			)
			.await
			.err(),
			Some(PdfError::Unavailable)
		);
		assert_eq!(read("", pdf, 0).await.err(), Some(PdfError::Limit));
		assert_eq!(
			read("", pdf, MAX_PDF_BYTES as u64 + 1).await.err(),
			Some(PdfError::Limit)
		);
	}
}
