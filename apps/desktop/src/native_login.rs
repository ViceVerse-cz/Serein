//! Native sign-in: Discord's own login form, MFA step and QR code, drawn by Serein and sent
//! with the identity every later request uses. A challenge is shown for the user to solve
//! and nothing is retried on its own. See `discord_api::login` and
//! `discord_gateway::remote_auth` for the wire side (unofficial, live-unverified).
//!
//! One attempt runs at a time. Submitting the form stops the QR code, and a scanned code
//! disables the form, so neither can answer the other's challenge or sign in a different
//! account behind it.
use super::*;
use client_core::captcha::{Challenge, Solved};
use discord_api::login::{Exchange, LoginApi, LoginError, MfaChallenge, MfaMethod, Step};
use discord_gateway::remote_auth::{Account, Progress, RemoteAuth, Stop};

/// Width of the email and password column, and of every other sign-in column.
pub(super) const FORM_WIDTH: f32 = 400.0;
const QR_WIDTH: f32 = 220.0;
/// Space between the form and the QR column; the divider runs down its middle.
const COLUMN_GAP: f32 = 64.0;
const QR_TILE: f32 = 176.0;
/// A code nobody used is replaced this many times before the user is asked to refresh,
/// so an unattended sign-in screen stops opening sockets.
const QR_RENEWALS: u8 = 4;
/// Discord keeps an unscanned code for about two minutes.
const QR_SHORTEST_LIFE: Duration = Duration::from_secs(30);
/// Room for a password of the longest length the field accepts, so typing never moves it.
const SECRET_BYTES: usize = 512;
/// Shown in fixture builds, where Discord is never contacted.
#[cfg(feature = "demo")]
const FIXTURE_CODE: &str = "https://discord.com/ra/synthetic-fixture-not-a-sign-in-code";

struct AbortTask(tokio::task::JoinHandle<()>);
impl Drop for AbortTask {
	fn drop(&mut self) {
		self.0.abort();
	}
}

enum Request {
	Login {
		login: String,
		password: Zeroizing<String>,
		solved: Option<Solved>,
	},
	Mfa {
		challenge: Arc<MfaChallenge>,
		method: MfaMethod,
		code: Zeroizing<String>,
	},
	Sms(Arc<MfaChallenge>),
}

/// What the form asks of the QR worker. Each start carries a new epoch, and the worker
/// stamps its events with it, so nothing from a stopped code is applied.
enum QrControl {
	Start(u64),
	Pause,
	Solved(u64, Box<Solved>),
}

enum QrEvent {
	Loading,
	Progress(Progress),
	/// The phone confirmed; the ticket is being traded for the session.
	Approved,
	Stopped(Stop),
	Rejected(LoginError),
	Captcha(Challenge),
	Token(SessionSecret),
}

enum Event {
	Step(Result<Step, LoginError>),
	Mfa(Result<SessionSecret, LoginError>),
	Sms(Result<(), LoginError>),
	Qr(u64, QrEvent),
}

/// The two workers behind an open sign-in form. Dropping it stops both.
struct Session {
	events: tokio::sync::mpsc::Receiver<Event>,
	requests: tokio::sync::mpsc::Sender<Request>,
	qr: tokio::sync::mpsc::Sender<QrControl>,
	_tasks: [AbortTask; 2],
}

#[derive(Default)]
enum Stage {
	#[default]
	Credentials,
	Mfa {
		challenge: Arc<MfaChallenge>,
		method: MfaMethod,
	},
}

#[derive(Default)]
enum Qr {
	/// No code is asked for; one starts as soon as the form is free.
	#[default]
	Off,
	Loading,
	Code(egui::TextureHandle),
	/// A phone scanned the code, which stays on screen, faded, behind its account.
	Scanned {
		code: Option<egui::TextureHandle>,
		account: Option<Account>,
		/// The phone confirmed; the session is on its way.
		approved: bool,
	},
	/// Stopped until the user asks for a new code.
	Expired,
	Failed(String),
}

/// Which request a solved challenge replays.
#[derive(Clone, Copy, PartialEq, Eq)]
enum CaptchaFor {
	Password,
	Qr,
}

struct CaptchaStep {
	challenge: Challenge,
	target: CaptchaFor,
	view: Option<platform::captcha::CaptchaView>,
	/// Whether opening the view was already tried, so a failure is not retried every frame.
	opened: bool,
}

#[derive(Default)]
pub(super) struct NativeLogin {
	/// Returning owners see their saved accounts first and open the form themselves.
	pub(super) open: bool,
	/// The folded-away way to Discord's hosted login page.
	pub(super) web_open: bool,
	login: String,
	password: Zeroizing<String>,
	code: Zeroizing<String>,
	stage: Stage,
	pub(super) busy: bool,
	/// Discord's rate limit: nothing is submitted before it.
	cooldown: Option<std::time::Instant>,
	error: Option<String>,
	notice: Option<String>,
	qr: Qr,
	qr_epoch: u64,
	captcha: Option<CaptchaStep>,
	session: Option<Session>,
	/// The frame that last drew the form; a later frame without it stops the workers.
	drawn: Option<u64>,
	focus_pending: bool,
	/// Last frame's column heights, to centre the QR code beside the form.
	form_height: f32,
	qr_height: f32,
}

impl NativeLogin {
	/// Stops the workers and forgets everything typed, including a half-finished MFA step.
	pub(super) fn close(&mut self) {
		*self = Self {
			open: self.open,
			web_open: self.web_open,
			login: std::mem::take(&mut self.login),
			cooldown: self.cooldown,
			..Self::default()
		};
	}
	pub(super) fn wide(&self) -> bool {
		matches!(self.stage, Stage::Credentials) && self.captcha.is_none()
	}
	fn cooling(&self) -> Option<Duration> {
		self.cooldown?
			.checked_duration_since(std::time::Instant::now())
			.filter(|left| !left.is_zero())
	}
	/// Whether the form's own attempt is under way, from submit to its last step.
	fn password_attempt(&self) -> bool {
		self.busy
			|| matches!(self.stage, Stage::Mfa { .. })
			|| self
				.captcha
				.as_ref()
				.is_some_and(|step| step.target == CaptchaFor::Password)
	}
	fn close_captcha(&mut self, target: CaptchaFor) {
		if self
			.captcha
			.as_ref()
			.is_some_and(|step| step.target == target)
		{
			self.captcha = None;
		}
	}
	/// Keeps the QR code running only while the form is free, so one attempt is live.
	fn sync_qr(&mut self) {
		let Some(session) = &self.session else {
			return;
		};
		let free = !self.password_attempt();
		match (&self.qr, free) {
			// A rate limit holds back the code as well: its exchange is the same service.
			(Qr::Off, true) if self.cooling().is_none() => {
				self.qr_epoch += 1;
				if session.qr.try_send(QrControl::Start(self.qr_epoch)).is_ok() {
					self.qr = Qr::Loading;
				}
			}
			(Qr::Loading | Qr::Code(_) | Qr::Scanned { .. }, false) => {
				// The epoch moves even if the worker cannot be told: its events are then stale.
				self.qr_epoch += 1;
				let _ = session.qr.try_send(QrControl::Pause);
				self.qr = Qr::Off;
				self.close_captcha(CaptchaFor::Qr);
			}
			_ => {}
		}
	}
	/// A new code on request: after an expiry or failure, or "Not me".
	fn restart_qr(&mut self) {
		self.close_captcha(CaptchaFor::Qr);
		self.qr = Qr::Off;
		// A worker that is gone cannot answer; the next frame starts both again.
		if self
			.session
			.as_ref()
			.is_some_and(|session| session.qr.is_closed())
		{
			self.stop_workers();
		}
		self.sync_qr();
	}
	fn rate_limited(&mut self, error: LoginError) {
		if let LoginError::RateLimited(seconds) = error {
			self.cooldown = Some(std::time::Instant::now() + Duration::from_secs(seconds.into()));
		}
	}
	/// A challenge left unsolved past its lifetime ends its attempt, also behind a hidden window.
	fn expire_captcha(&mut self) {
		let Some(step) = self.captcha.take_if(|step| {
			step.view
				.as_ref()
				.is_some_and(platform::captcha::CaptchaView::expired)
		}) else {
			return;
		};
		self.error = Some("Verification expired. Start the check again.".to_owned());
		match step.target {
			CaptchaFor::Password => self.password = Zeroizing::default(),
			CaptchaFor::Qr => self.restart_qr(),
		}
	}
	fn fail(&mut self, error: LoginError) {
		self.busy = false;
		self.notice = None;
		// A refused password is typed again; it is not kept waiting in the form.
		self.password = Zeroizing::default();
		self.rate_limited(error);
		if error == LoginError::TicketExpired {
			self.stage = Stage::Credentials;
		}
		self.code = Zeroizing::default();
		self.focus_pending = true;
		self.error = Some(error.to_string());
	}
	fn submit(&mut self, request: Request) {
		if self.cooling().is_some() {
			return;
		}
		let sent = self
			.session
			.as_ref()
			.is_some_and(|session| session.requests.try_send(request).is_ok());
		self.busy = sent;
		self.notice = None;
		self.error = (!sent).then(|| "Sign-in is busy. Try again in a moment.".to_owned());
		// From here the form's attempt is the only one.
		self.sync_qr();
	}
	fn submit_login(&mut self, solved: Option<Solved>) {
		let login = self.login.trim().to_owned();
		if login.is_empty() || self.password.is_empty() {
			return;
		}
		self.submit(Request::Login {
			login,
			password: self.password.clone(),
			solved,
		});
	}
	fn submit_code(&mut self) {
		let Stage::Mfa { challenge, method } = &self.stage else {
			return;
		};
		let request = Request::Mfa {
			challenge: challenge.clone(),
			method: *method,
			code: self.code.clone(),
		};
		self.submit(request);
	}
	fn choose_method(&mut self, chosen: MfaMethod) {
		let Stage::Mfa { challenge, method } = &mut self.stage else {
			return;
		};
		*method = chosen;
		let challenge = challenge.clone();
		self.code = Zeroizing::default();
		self.error = None;
		self.notice = None;
		self.focus_pending = true;
		if chosen == MfaMethod::Sms {
			self.submit(Request::Sms(challenge));
		}
	}
	fn back_to_credentials(&mut self) {
		self.stage = Stage::Credentials;
		self.code = Zeroizing::default();
		self.error = None;
		self.notice = None;
		self.focus_pending = true;
	}
	fn start(
		&mut self,
		runtime: &tokio::runtime::Handle,
		proxy: tokio::sync::watch::Receiver<Option<discord_api::proxy::ApiProxy>>,
		ctx: &egui::Context,
	) {
		let api = match LoginApi::new(proxy) {
			Ok(api) => Arc::new(api),
			Err(failure) => {
				self.error = Some(failure.label().to_owned());
				return;
			}
		};
		let (send, events) = tokio::sync::mpsc::channel(16);
		let (requests, incoming) = tokio::sync::mpsc::channel(1);
		let (qr, control) = tokio::sync::mpsc::channel(4);
		let worker = QrWorker {
			api: api.clone(),
			control,
			events: send.clone(),
			ctx: ctx.clone(),
		};
		let tasks = [
			AbortTask(runtime.spawn(run_requests(api, incoming, send, ctx.clone()))),
			AbortTask(runtime.spawn(worker.run())),
		];
		self.qr = Qr::Off;
		self.session = Some(Session {
			events,
			requests,
			qr,
			_tasks: tasks,
		});
	}
	/// The form left the screen: both workers stop, and what they were in the middle of is
	/// dropped with the secrets it needed. A reached MFA step stays, since its code is not
	/// tied to these workers.
	fn stop_workers(&mut self) {
		self.session = None;
		self.captcha = None;
		self.busy = false;
		self.qr = Qr::Off;
		self.password = Zeroizing::default();
		self.code = Zeroizing::default();
	}
	/// Applies what the workers reported; returns the session once Discord issued one.
	pub(super) fn poll(&mut self, ctx: &egui::Context) -> Option<SessionSecret> {
		// Frames, not calls: a hidden window runs this without drawing, and must not look
		// like a frame that left the form out.
		if self
			.drawn
			.is_some_and(|frame| ctx.cumulative_frame_nr() > frame + 1)
		{
			self.drawn = None;
			self.stop_workers();
			return None;
		}
		self.expire_captcha();
		// A worker that ended on its own would leave the code loading for good.
		if matches!(self.qr, Qr::Loading | Qr::Code(_) | Qr::Scanned { .. })
			&& self
				.session
				.as_ref()
				.is_some_and(|session| session.qr.is_closed())
		{
			self.close_captcha(CaptchaFor::Qr);
			self.qr = Qr::Failed(String::new());
		}
		loop {
			let event = self.session.as_mut()?.events.try_recv().ok()?;
			match event {
				Event::Step(Ok(Step::Token(secret))) | Event::Mfa(Ok(secret)) => {
					return Some(secret);
				}
				Event::Step(Ok(Step::Mfa(challenge))) => {
					self.busy = false;
					self.error = None;
					self.password = Zeroizing::default();
					self.code = Zeroizing::default();
					self.focus_pending = true;
					let method = [MfaMethod::Totp, MfaMethod::Backup, MfaMethod::Sms]
						.into_iter()
						.find(|method| challenge.allows(*method))
						.unwrap_or(MfaMethod::Totp);
					self.stage = Stage::Mfa {
						challenge: Arc::new(challenge),
						method,
					};
					if method == MfaMethod::Sms {
						self.choose_method(method);
					}
				}
				Event::Step(Ok(Step::Captcha(challenge))) => {
					self.busy = false;
					self.error = None;
					self.captcha = Some(CaptchaStep {
						challenge,
						target: CaptchaFor::Password,
						view: None,
						opened: false,
					});
				}
				Event::Step(Err(error)) | Event::Mfa(Err(error)) | Event::Sms(Err(error)) => {
					self.fail(error);
				}
				Event::Sms(Ok(())) => {
					self.busy = false;
					self.notice = Some(ui::i18n::translate("native-login-sms-sent"));
				}
				// From a code that was stopped or replaced since.
				Event::Qr(epoch, _) if epoch != self.qr_epoch => {}
				Event::Qr(_, QrEvent::Token(secret)) => return Some(secret),
				Event::Qr(_, QrEvent::Loading) => {
					self.close_captcha(CaptchaFor::Qr);
					self.qr = Qr::Loading;
				}
				Event::Qr(_, QrEvent::Progress(Progress::Code(address))) => {
					self.qr = qr_texture(ctx, &address)
						.map_or_else(|| Qr::Failed(String::new()), Qr::Code);
				}
				Event::Qr(_, QrEvent::Progress(Progress::Scanned(account))) => {
					// The phone's account takes over; nothing typed stays behind it.
					self.password = Zeroizing::default();
					self.error = None;
					let code = match std::mem::take(&mut self.qr) {
						Qr::Code(code) => Some(code),
						_ => None,
					};
					self.qr = Qr::Scanned {
						code,
						account,
						approved: false,
					};
				}
				Event::Qr(_, QrEvent::Approved) => {
					if let Qr::Scanned { approved, .. } = &mut self.qr {
						*approved = true;
					}
				}
				Event::Qr(_, QrEvent::Captcha(challenge)) => {
					self.captcha = Some(CaptchaStep {
						challenge,
						target: CaptchaFor::Qr,
						view: None,
						opened: false,
					});
				}
				Event::Qr(_, QrEvent::Stopped(stop)) => {
					self.close_captcha(CaptchaFor::Qr);
					self.qr = match stop {
						Stop::Failed(label) => Qr::Failed(label.to_owned()),
						Stop::Expired | Stop::Cancelled => Qr::Expired,
					};
				}
				Event::Qr(_, QrEvent::Rejected(error)) => {
					self.close_captcha(CaptchaFor::Qr);
					self.rate_limited(error);
					self.qr = Qr::Failed(error.to_string());
				}
			}
		}
	}
	/// Fixture-only: puts the form into one of its later states for a capture.
	#[cfg(feature = "demo")]
	pub(super) fn preview(&mut self, state: &str) {
		self.open = true;
		self.login = "ada@example.com".to_owned();
		match state {
			"mfa" => {
				self.stage = Stage::Mfa {
					challenge: Arc::new(discord_api::login::MfaChallenge::fixture()),
					method: MfaMethod::Totp,
				};
			}
			"scanned" => {
				self.qr = Qr::Scanned {
					code: None,
					account: Some(Account {
						id: model::Id(1),
						name: "Ada Lovelace".to_owned(),
						discriminator: "0".to_owned(),
						avatar: None,
					}),
					approved: false,
				};
			}
			"error" => self.error = Some(LoginError::NewLocation.to_string()),
			"expired" => self.qr = Qr::Expired,
			_ => {}
		}
	}
}

async fn run_requests(
	api: Arc<LoginApi>,
	mut requests: tokio::sync::mpsc::Receiver<Request>,
	events: tokio::sync::mpsc::Sender<Event>,
	ctx: egui::Context,
) {
	api.open_page().await;
	while let Some(request) = requests.recv().await {
		let event = match request {
			Request::Login {
				login,
				password,
				solved,
			} => Event::Step(api.login(&login, &password, solved.as_ref()).await),
			Request::Mfa {
				challenge,
				method,
				code,
			} => Event::Mfa(api.mfa(&challenge, method, &code).await),
			Request::Sms(challenge) => Event::Sms(api.send_sms(&challenge).await),
		};
		if events.send(event).await.is_err() {
			return;
		}
		ctx.request_repaint();
	}
}

/// How one QR code ended.
enum Ended {
	Token,
	Stopped(Stop),
	Rejected(LoginError),
	/// The form asked for something else first; `None` when the form is gone.
	Control(Option<QrControl>),
}

/// Waits for `work` unless the form starts or pauses a code first. A challenge answer that
/// arrives while none is awaited belongs to a code that is gone, and is dropped.
async fn unless_asked<T>(
	control: &mut tokio::sync::mpsc::Receiver<QrControl>,
	work: impl Future<Output = T>,
) -> Result<T, Option<QrControl>> {
	tokio::pin!(work);
	loop {
		tokio::select! {
			done = &mut work => return Ok(done),
			request = control.recv() => match request {
				Some(QrControl::Solved(..)) => {}
				other => return Err(other),
			},
		}
	}
}

struct QrWorker {
	api: Arc<LoginApi>,
	control: tokio::sync::mpsc::Receiver<QrControl>,
	events: tokio::sync::mpsc::Sender<Event>,
	ctx: egui::Context,
}
impl QrWorker {
	async fn emit(&self, epoch: u64, event: QrEvent) -> bool {
		let sent = self.events.send(Event::Qr(epoch, event)).await.is_ok();
		self.ctx.request_repaint();
		sent
	}
	/// Shows one code after another while the form asks for them. A code nobody used is
	/// renewed within a budget; after that, a failure or a refusal, nothing more is opened
	/// until the form asks again.
	async fn run(mut self) {
		let mut epoch = 0;
		let mut renewals = 0;
		let mut running = false;
		loop {
			if !running {
				match self.control.recv().await {
					Some(QrControl::Start(next)) => {
						epoch = next;
						renewals = 0;
						running = true;
					}
					Some(QrControl::Pause | QrControl::Solved(..)) => continue,
					None => return,
				}
			}
			let started = tokio::time::Instant::now();
			let report = match self.code(epoch).await {
				Ended::Control(None) => return,
				// The form may have moved on while the answer arrived; the worker stays for
				// its next request instead of leaving a new code unanswered.
				Ended::Token => {
					running = false;
					continue;
				}
				Ended::Control(Some(QrControl::Start(next))) => {
					epoch = next;
					renewals = 0;
					continue;
				}
				Ended::Control(Some(QrControl::Pause | QrControl::Solved(..))) => {
					running = false;
					continue;
				}
				// The owner declined on the phone: a new code, as Discord's page shows one.
				Ended::Stopped(Stop::Cancelled) if renewals < QR_RENEWALS => None,
				// A code Discord closes at once is a refusal, not an expiry.
				Ended::Stopped(Stop::Expired)
					if renewals < QR_RENEWALS && started.elapsed() >= QR_SHORTEST_LIFE =>
				{
					None
				}
				Ended::Stopped(stop) => Some(QrEvent::Stopped(stop)),
				Ended::Rejected(error) => Some(QrEvent::Rejected(error)),
			};
			match report {
				None => renewals += 1,
				Some(event) => {
					if !self.emit(epoch, event).await {
						return;
					}
					running = false;
				}
			}
		}
	}
	/// One code: the socket until a phone approves it, then the ticket for the session.
	async fn code(&mut self, epoch: u64) -> Ended {
		if !self.emit(epoch, QrEvent::Loading).await {
			return Ended::Control(None);
		}
		let (events, ctx) = (self.events.clone(), self.ctx.clone());
		let approval = async {
			let auth = tokio::task::spawn_blocking(RemoteAuth::new)
				.await
				.map_err(|_| Stop::Failed("Could not create a key for QR sign-in."))??;
			let progress = |progress| {
				// Two per code at most; a full queue means the form is gone.
				let _ = events.try_send(Event::Qr(epoch, QrEvent::Progress(progress)));
				ctx.request_repaint();
			};
			let ticket = auth.approve(progress).await?;
			Ok::<_, Stop>((auth, ticket))
		};
		let (auth, ticket) = match unless_asked(&mut self.control, approval).await {
			Ok(Ok(approved)) => approved,
			Ok(Err(stop)) => return Ended::Stopped(stop),
			Err(request) => return Ended::Control(request),
		};
		if !self.emit(epoch, QrEvent::Approved).await {
			return Ended::Control(None);
		}
		let mut solved = None;
		loop {
			// "Not me" still stops it here: the answer of a stopped code is never used.
			let exchange = self.api.remote_auth(&ticket, solved.as_ref());
			let answer = match unless_asked(&mut self.control, exchange).await {
				Ok(answer) => answer,
				Err(request) => return Ended::Control(request),
			};
			match answer {
				Ok(Exchange::Encrypted(token)) => {
					let Some(secret) = auth.open_token(&token) else {
						return Ended::Stopped(Stop::Failed(
							"Discord approved the QR sign-in, but its answer could not be opened.",
						));
					};
					self.emit(epoch, QrEvent::Token(secret)).await;
					return Ended::Token;
				}
				Ok(Exchange::Captcha(challenge)) => {
					if !self.emit(epoch, QrEvent::Captcha(challenge)).await {
						return Ended::Control(None);
					}
					let answer = async {
						loop {
							match self.control.recv().await {
								Some(QrControl::Solved(from, answer)) if from == epoch => {
									return Ok(answer);
								}
								Some(QrControl::Solved(..)) => {}
								other => return Err(other),
							}
						}
					};
					let wait = tokio::time::timeout(client_core::captcha::LIFETIME, answer);
					match wait.await {
						Ok(Ok(answer)) => solved = Some(*answer),
						Ok(Err(request)) => return Ended::Control(request),
						Err(_) => {
							return Ended::Stopped(Stop::Failed(
								"Verification expired. Refresh the code to try again.",
							));
						}
					}
				}
				Err(error) => return Ended::Rejected(error),
			}
		}
	}
}

/// One texel per module with a quiet zone, scaled without filtering so the code stays sharp.
fn qr_texture(ctx: &egui::Context, address: &str) -> Option<egui::TextureHandle> {
	// The highest correction level leaves room for the mark drawn over the middle.
	let code = qrcodegen::QrCode::encode_text(address, qrcodegen::QrCodeEcc::High).ok()?;
	const QUIET: i32 = 2;
	let side = code.size() + QUIET * 2;
	let mut pixels = Vec::with_capacity((side * side) as usize);
	for y in 0..side {
		for x in 0..side {
			pixels.push(if code.get_module(x - QUIET, y - QUIET) {
				egui::Color32::BLACK
			} else {
				egui::Color32::WHITE
			});
		}
	}
	Some(ctx.load_texture(
		"native-login-qr",
		egui::ColorImage::new([side as usize; 2], pixels),
		egui::TextureOptions::NEAREST,
	))
}

/// Link-coloured inline action, as the links on Discord's own login form.
fn link(ui: &mut egui::Ui, text: &str) -> egui::Response {
	let p = ui::design::palette(ui);
	let response = ui.add(
		egui::Label::new(ui::design::medium(ui, text, 13.0).color(p.link))
			.selectable(false)
			.sense(egui::Sense::click()),
	);
	response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Link, ui.is_enabled(), text));
	if response.hovered() && ui.is_enabled() {
		ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
		let y = response.rect.bottom() - 1.0;
		ui.painter()
			.hline(response.rect.x_range(), y, egui::Stroke::new(1.0, p.link));
	}
	response
}

/// A field for a password or code. egui keeps undo snapshots of a field's text, also for a
/// password field; they are dropped every frame so nothing typed here outlives the field.
fn secret_input(ui: &mut egui::Ui, edit: egui::TextEdit<'_>) -> egui::Response {
	let response = ui::design::input(ui, edit);
	if let Some(mut state) = egui::TextEdit::load_state(ui.ctx(), response.id) {
		state.clear_undoer();
		state.store(ui.ctx(), response.id);
	}
	response
}

fn pressed_enter(ui: &egui::Ui, field: &egui::Response) -> bool {
	field.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter))
}

impl Desktop {
	/// Title and subtitle for the step the native form is on.
	fn native_login_heading(&self, returning: bool) -> (&'static str, &'static str) {
		if self.native.captcha.is_some() {
			return (
				"native-login-captcha-title",
				"native-login-captcha-subtitle",
			);
		}
		match &self.native.stage {
			Stage::Credentials => (
				if returning {
					"main-sign-in-header-welcome-back"
				} else {
					"main-sign-in-header-welcome-to-serein"
				},
				"main-sign-in-header-sign-in-with-your-discord-account-to-get-started",
			),
			Stage::Mfa { method, .. } => (
				"native-login-mfa-title",
				match method {
					MfaMethod::Totp => "native-login-mfa-totp-subtitle",
					MfaMethod::Sms => "native-login-mfa-sms-subtitle",
					MfaMethod::Backup => "native-login-mfa-backup-subtitle",
				},
			),
		}
	}
	/// The native form for the step it is on, inside the sign-in card. `wide` puts the QR
	/// code beside the fields.
	pub(super) fn native_login_form(&mut self, ui: &mut egui::Ui, wide: bool, returning: bool) {
		self.native.drawn = Some(ui.ctx().cumulative_frame_nr());
		if self.native.session.is_none() && !self.fixture_only {
			self.native
				.start(self.runtime.handle(), self.api_proxy.subscribe(), ui.ctx());
			self.native.focus_pending = true;
		}
		self.native.sync_qr();
		#[cfg(feature = "demo")]
		if self.fixture_only {
			match &mut self.native.qr {
				Qr::Off => {
					self.native.qr =
						qr_texture(ui.ctx(), FIXTURE_CODE).map_or(Qr::Expired, Qr::Code);
				}
				Qr::Scanned { code, .. } if code.is_none() => {
					*code = qr_texture(ui.ctx(), FIXTURE_CODE);
				}
				_ => {}
			}
		}
		if let Some(left) = self.native.cooling() {
			ui.ctx().request_repaint_after(left);
		}
		let heading = self.native_login_heading(returning);
		// A scanned code is the attempt in progress; the fields wait behind it.
		let scanned = matches!(self.native.qr, Qr::Scanned { .. });
		if self.native.captcha.is_some() {
			form_heading(ui, heading);
			self.native_login_captcha(ui);
		} else if matches!(self.native.stage, Stage::Mfa { .. }) {
			form_heading(ui, heading);
			self.native_login_mfa(ui);
		} else if wide {
			let row = ui
				.horizontal_top(|ui| {
					ui.spacing_mut().item_spacing.x = 0.0;
					let form = ui
						.allocate_ui_with_layout(
							egui::vec2(FORM_WIDTH, 0.0),
							egui::Layout::top_down(egui::Align::Min),
							|ui| {
								ui.add_enabled_ui(!scanned, |ui| {
									form_heading(ui, heading);
									self.native_login_credentials(ui);
								});
							},
						)
						.response
						.rect
						.height();
					ui.add_space(COLUMN_GAP);
					// Centred on the form beside it, from the heights of the frame before.
					let top = ((self.native.form_height - self.native.qr_height) * 0.5).max(0.0);
					let qr = ui
						.allocate_ui_with_layout(
							egui::vec2(QR_WIDTH, 0.0),
							egui::Layout::top_down(egui::Align::Center),
							|ui| {
								ui.add_space(top);
								self.native_login_qr(ui);
							},
						)
						.response
						.rect
						.height();
					self.native.form_height = form;
					self.native.qr_height = qr - top;
				})
				.response
				.rect;
			ui.painter().vline(
				row.left() + FORM_WIDTH + COLUMN_GAP * 0.5,
				row.y_range(),
				egui::Stroke::new(1.0, ui::design::palette(ui).border),
			);
		} else {
			ui.add_enabled_ui(!scanned, |ui| {
				form_heading(ui, heading);
				self.native_login_credentials(ui);
			});
			ui.add_space(24.0);
			or_divider(ui);
			ui.add_space(24.0);
			ui.vertical_centered(|ui| self.native_login_qr(ui));
		}
	}
	fn native_login_messages(&self, ui: &mut egui::Ui) {
		if let Some(error) = &self.native.error {
			ui::design::notice(ui, ui::design::Level::Error, error);
			ui.add_space(16.0);
		} else if let Some(notice) = &self.native.notice {
			ui::design::notice(ui, ui::design::Level::Info, notice);
			ui.add_space(16.0);
		}
	}
	fn native_login_credentials(&mut self, ui: &mut egui::Ui) {
		let p = ui::design::palette(ui);
		self.native_login_messages(ui);
		let typing = !self.native.busy;
		let idle = typing && self.native.cooling().is_none();
		let mut submit = false;
		if self.native.password.capacity() < SECRET_BYTES {
			self.native.password.reserve(SECRET_BYTES);
		}
		ui.add_enabled_ui(typing, |ui| {
			ui::design::label(ui, "native-login-email-or-phone");
			let login = ui::design::input(
				ui,
				egui::TextEdit::singleline(&mut self.native.login)
					.id_salt("native-login-email")
					.char_limit(254),
			);
			ui.add_space(16.0);
			ui::design::label(ui, "native-login-password");
			let password = secret_input(
				ui,
				egui::TextEdit::singleline(&mut *self.native.password)
					.id_salt("native-login-password")
					.password(true)
					.char_limit(128),
			);
			// The first field still to fill in.
			if std::mem::take(&mut self.native.focus_pending) {
				if self.native.login.trim().is_empty() {
					login.request_focus();
				} else {
					password.request_focus();
				}
			}
			submit = pressed_enter(ui, &login) || pressed_enter(ui, &password);
		});
		ui.add_space(8.0);
		if link(ui, &ui::i18n::translate("native-login-forgot-password")).clicked() {
			ui.ctx()
				.open_url(egui::OpenUrl::new_tab("https://discord.com/login"));
		}
		ui.add_space(20.0);
		let ready =
			idle && !self.native.login.trim().is_empty() && !self.native.password.is_empty();
		let label = ui::i18n::translate(if self.native.busy {
			"native-login-logging-in"
		} else {
			"native-login-log-in"
		});
		// Enabled even when a field is empty, as on Discord's form; the click then goes there.
		let clicked = ui
			.add_enabled_ui(idle, |ui| ui::design::primary_button(ui, &label))
			.inner
			.clicked();
		self.native.focus_pending |= clicked && !ready;
		submit |= clicked;
		ui.add_space(12.0);
		ui.horizontal(|ui| {
			ui.spacing_mut().item_spacing.x = 4.0;
			ui.label(
				egui::RichText::new(ui::i18n::translate("native-login-need-an-account"))
					.size(13.0)
					.color(p.muted),
			);
			if link(ui, &ui::i18n::translate("native-login-register")).clicked() {
				ui.ctx()
					.open_url(egui::OpenUrl::new_tab("https://discord.com/register"));
			}
		});
		if submit && ready && !self.fixture_only {
			self.native.submit_login(None);
		}
	}
	fn native_login_mfa(&mut self, ui: &mut egui::Ui) {
		let Stage::Mfa { challenge, method } = &self.native.stage else {
			return;
		};
		let (challenge, method) = (challenge.clone(), *method);
		self.native_login_messages(ui);
		let typing = !self.native.busy;
		let idle = typing && self.native.cooling().is_none();
		let (label, hint, length) = match method {
			MfaMethod::Totp => (
				"native-login-mfa-totp-label",
				"native-login-mfa-six-digits",
				6,
			),
			MfaMethod::Sms => (
				"native-login-mfa-sms-label",
				"native-login-mfa-six-digits",
				6,
			),
			MfaMethod::Backup => (
				"native-login-mfa-backup-label",
				"native-login-mfa-eight-characters",
				8,
			),
		};
		let mut submit = false;
		if self.native.code.capacity() < SECRET_BYTES {
			self.native.code.reserve(SECRET_BYTES);
		}
		ui.add_enabled_ui(typing, |ui| {
			ui::design::label(ui, label);
			let code = secret_input(
				ui,
				egui::TextEdit::singleline(&mut *self.native.code)
					.id_salt("native-login-code")
					.char_limit(12)
					.hint_text(ui::i18n::translate(hint)),
			);
			if std::mem::take(&mut self.native.focus_pending) {
				code.request_focus();
			}
			submit = pressed_enter(ui, &code);
		});
		ui.add_space(20.0);
		let entered = self
			.native
			.code
			.chars()
			.filter(char::is_ascii_alphanumeric)
			.count();
		let ready = idle && entered == length;
		let action = ui::i18n::translate(if self.native.busy {
			"native-login-logging-in"
		} else {
			"native-login-log-in"
		});
		let clicked = ui
			.add_enabled_ui(idle, |ui| ui::design::primary_button(ui, &action))
			.inner
			.clicked();
		self.native.focus_pending |= clicked && !ready;
		submit |= clicked;
		ui.add_space(16.0);
		let mut chosen = None;
		let mut back = false;
		ui.add_enabled_ui(idle, |ui| {
			ui.spacing_mut().item_spacing.y = 8.0;
			for (other, key) in [
				(MfaMethod::Totp, "native-login-mfa-use-authenticator"),
				(MfaMethod::Sms, "native-login-mfa-use-sms"),
				(MfaMethod::Backup, "native-login-mfa-use-backup"),
			] {
				if other != method
					&& challenge.allows(other)
					&& link(ui, &ui::i18n::translate(key)).clicked()
				{
					chosen = Some(other);
				}
			}
			if method == MfaMethod::Sms
				&& link(ui, &ui::i18n::translate("native-login-mfa-resend-sms")).clicked()
			{
				chosen = Some(MfaMethod::Sms);
			}
			back = link(ui, &ui::i18n::translate("native-login-back-to-login")).clicked();
		});
		if self.fixture_only {
			return;
		}
		if back {
			self.native.back_to_credentials();
		} else if let Some(chosen) = chosen {
			self.native.choose_method(chosen);
		} else if submit && ready {
			self.native.submit_code();
		}
	}
	/// Discord's hCaptcha, shown only when a sign-in answer asks for it and solved by the user.
	fn native_login_captcha(&mut self, ui: &mut egui::Ui) {
		let p = ui::design::palette(ui);
		let height = (ui.ctx().content_rect().height() - 320.0).clamp(200.0, 520.0);
		let (rect, _) = ui.allocate_exact_size(
			egui::vec2(ui.available_width(), height),
			egui::Sense::hover(),
		);
		ui.painter().rect_filled(rect, 10, p.base);
		ui.put(
			egui::Rect::from_center_size(rect.center(), egui::Vec2::splat(20.0)),
			egui::Spinner::new().size(18.0).color(p.muted),
		);
		ui.add_space(12.0);
		let cancel = ui
			.vertical_centered(|ui| {
				ui::design::text_action(ui, &ui::i18n::translate("native-login-captcha-cancel"))
			})
			.inner
			.clicked();
		let ctx = ui.ctx().clone();
		let dark = ctx.theme() == egui::Theme::Dark;
		let route = self.api_proxy.borrow().clone();
		let Some(step) = &mut self.native.captcha else {
			return;
		};
		let mut failed = None;
		if !step.opened && !self.fixture_only {
			step.opened = true;
			let wake = ctx.clone();
			match platform::captcha::CaptchaView::open(
				self.window.clone(),
				&step.challenge,
				dark,
				captcha::webview_proxy(route.as_ref()).as_ref(),
				move || wake.request_repaint(),
			) {
				Ok(view) => step.view = Some(view),
				Err(error) => failed = Some(error),
			}
		}
		let mut solution = None;
		if let Some(view) = &step.view {
			// A native child is not clipped by the scroll area: scrolled partly out of view, it
			// would cover the title bar or footer, so it is hidden until it fits again.
			if ui.clip_rect().contains_rect(rect) {
				let scale = ctx.pixels_per_point();
				view.set_bounds(
					(rect.left() * scale).round() as i32,
					(rect.top() * scale).round() as i32,
					(rect.width() * scale).round() as u32,
					(rect.height() * scale).round() as u32,
				);
			} else {
				view.set_bounds(0, 0, 0, 0);
			}
			ctx.request_repaint_after(if cfg!(target_os = "linux") {
				Duration::from_millis(20)
			} else {
				Duration::from_secs(1)
			});
			if view.expired() {
				failed = Some("Verification expired. Start the check again.");
			} else {
				match view.poll() {
					Some(Ok(answer)) => solution = Some(answer),
					Some(Err(error)) => failed = Some(error),
					None => {}
				}
			}
		}
		if solution.is_none() && failed.is_none() && !cancel {
			return;
		}
		let Some(step) = self.native.captcha.take() else {
			return;
		};
		let solved = solution.map(|answer| Solved::new(step.challenge, answer));
		match (step.target, solved) {
			(CaptchaFor::Password, Some(solved)) => self.native.submit_login(Some(solved)),
			(CaptchaFor::Qr, Some(solved)) => {
				let answer = QrControl::Solved(self.native.qr_epoch, Box::new(solved));
				let sent = self
					.native
					.session
					.as_ref()
					.is_some_and(|session| session.qr.try_send(answer).is_ok());
				if !sent {
					self.native.restart_qr();
				}
			}
			(CaptchaFor::Qr, None) => {
				self.native.restart_qr();
				self.native.error = failed.map(str::to_owned);
			}
			(CaptchaFor::Password, None) => {
				// The attempt is over, so its password does not stay in the form.
				self.native.password = Zeroizing::default();
				self.native.focus_pending = true;
				self.native.error = failed.map(str::to_owned);
			}
		}
	}
	fn native_login_qr(&mut self, ui: &mut egui::Ui) {
		let p = ui::design::palette(ui);
		let whole = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
		let (tile, _) = ui.allocate_exact_size(egui::Vec2::splat(QR_TILE), egui::Sense::hover());
		// White with black modules whatever the theme: a phone camera has to read it.
		let fill = match &self.native.qr {
			Qr::Code(_) | Qr::Scanned { code: Some(_), .. } => egui::Color32::WHITE,
			_ => p.raised,
		};
		ui.painter().rect(
			tile,
			12,
			fill,
			egui::Stroke::new(1.0, p.border),
			egui::StrokeKind::Inside,
		);
		let mut refresh = false;
		match &self.native.qr {
			Qr::Code(texture) => {
				ui.painter()
					.image(texture.id(), tile.shrink(6.0), whole, egui::Color32::WHITE);
				let mark = egui::Rect::from_center_size(tile.center(), egui::Vec2::splat(34.0));
				ui.painter()
					.rect_filled(mark.expand(4.0), 10, egui::Color32::WHITE);
				ui.painter().rect_filled(mark, 8, p.accent);
				ui::icons::paint(
					ui.painter(),
					ui::icons::Icon::Serein,
					mark.shrink(8.0),
					p.accent_text,
				);
			}
			// As on Discord's page: the code fades behind whoever scanned it, and a ring turns
			// around them until the phone has answered.
			Qr::Scanned { code, account, .. } => {
				let disc = if let Some(texture) = code {
					ui.painter().image(
						texture.id(),
						tile.shrink(6.0),
						whole,
						egui::Color32::from_white_alpha(36),
					);
					egui::Color32::WHITE
				} else {
					p.raised
				};
				ui.painter().circle_filled(tile.center(), 52.0, disc);
				// Painted in place: the tile is already laid out, and the text follows it.
				egui::Spinner::new().color(p.accent).paint_at(
					ui,
					egui::Rect::from_center_size(tile.center(), egui::Vec2::splat(92.0)),
				);
				let avatar = egui::Rect::from_center_size(tile.center(), egui::Vec2::splat(68.0));
				let name = account
					.as_ref()
					.map_or("?", |account| account.name.as_str());
				ui::design::paint_avatar(ui, name, avatar.width(), avatar);
			}
			Qr::Off | Qr::Loading => {
				egui::Spinner::new().color(p.muted).paint_at(
					ui,
					egui::Rect::from_center_size(tile.center(), egui::Vec2::splat(22.0)),
				);
			}
			Qr::Expired | Qr::Failed(_) => {
				let button = egui::Rect::from_center_size(tile.center(), egui::vec2(120.0, 36.0));
				let response = ui.interact(
					button,
					ui.scope_id().with("qr-refresh"),
					egui::Sense::click(),
				);
				let label = ui::i18n::translate("native-login-qr-refresh");
				response
					.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &label));
				let fill = if response.hovered() {
					p.accent.linear_multiply(1.12)
				} else {
					p.accent
				};
				ui.painter().rect_filled(button, 8, fill);
				ui.painter().text(
					button.center(),
					egui::Align2::CENTER_CENTER,
					label,
					egui::FontId::new(14.0, ui::design::medium_family(ui.ctx())),
					p.accent_text,
				);
				if response.hovered() {
					ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
				}
				refresh = response.clicked();
			}
		}
		let (title, detail, undo) = match &self.native.qr {
			Qr::Scanned {
				account, approved, ..
			} => (
				if *approved {
					"native-login-logging-in"
				} else {
					"native-login-qr-check-your-phone"
				},
				match account {
					Some(account) => ui::i18n::translate_args(
						"native-login-qr-logging-in-as",
						&[("name", &account.name)],
					),
					None => ui::i18n::translate("native-login-qr-confirm-on-phone"),
				},
				!*approved,
			),
			Qr::Expired => (
				"native-login-qr-title",
				ui::i18n::translate("native-login-qr-expired"),
				false,
			),
			Qr::Failed(reason) if !reason.is_empty() => {
				("native-login-qr-title", reason.clone(), false)
			}
			Qr::Failed(_) => (
				"native-login-qr-title",
				ui::i18n::translate("native-login-qr-unavailable"),
				false,
			),
			_ => (
				"native-login-qr-title",
				ui::i18n::translate("native-login-qr-detail"),
				false,
			),
		};
		ui.add_space(20.0);
		ui.label(ui::design::semibold(ui, ui::i18n::translate(title), 20.0).color(p.text_strong));
		ui.add_space(8.0);
		ui.add(egui::Label::new(egui::RichText::new(detail).size(14.0).color(p.muted)).wrap());
		if undo {
			ui.add_space(8.0);
			if link(ui, &ui::i18n::translate("native-login-qr-not-me")).clicked()
				&& !self.fixture_only
			{
				self.native.restart_qr();
			}
		}
		if refresh && !self.fixture_only && self.native.cooling().is_none() {
			self.native.restart_qr();
		}
	}
}

/// The card's title and the line under it, centred over the column they head.
fn form_heading(ui: &mut egui::Ui, (title, subtitle): (&str, &str)) {
	let p = ui::design::palette(ui);
	ui.vertical_centered(|ui| {
		ui.label(ui::design::semibold(ui, ui::i18n::translate(title), 24.0).color(p.text_strong));
		ui.add_space(6.0);
		ui.add(
			egui::Label::new(
				egui::RichText::new(ui::i18n::translate(subtitle))
					.size(15.0)
					.color(p.muted),
			)
			.wrap(),
		);
	});
	ui.add_space(24.0);
}

/// A rule with "or" in its middle, between two ways to do the same thing.
pub(super) fn or_divider(ui: &mut egui::Ui) {
	let p = ui::design::palette(ui);
	let width = ui.available_width();
	let (rect, _) = ui.allocate_exact_size(egui::vec2(width, 16.0), egui::Sense::hover());
	let galley = ui.painter().layout_no_wrap(
		ui::i18n::translate("native-login-or"),
		egui::FontId::proportional(12.0),
		p.muted,
	);
	let gap = galley.size().x + 20.0;
	let y = rect.center().y;
	let stroke = egui::Stroke::new(1.0, p.border);
	ui.painter()
		.hline(rect.left()..=rect.center().x - gap * 0.5, y, stroke);
	ui.painter()
		.hline(rect.center().x + gap * 0.5..=rect.right(), y, stroke);
	ui.painter()
		.galley(rect.center() - galley.size() * 0.5, galley, p.muted);
}
