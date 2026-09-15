//! A temporary ephemeral GTK4/WebKit6 window for human-operated verification.
use super::{Challenge, Solution, page, parse_result};
use std::{
	cell::{Cell, RefCell},
	rc::Rc,
	sync::Arc,
	time::{Duration, Instant},
};
use webkit6::{gio, glib, prelude::*};

const LIFETIME: Duration = Duration::from_secs(300);
const QUERY_INTERVAL: Duration = Duration::from_millis(100);
const PAGE: &str = "https://serein-captcha.verification.invalid/";

struct Handoff {
	opened: Instant,
	closed: Cell<bool>,
	querying: Cell<bool>,
	delivered: Cell<bool>,
	last_query: Cell<Instant>,
	result: RefCell<Option<Result<Solution, &'static str>>>,
	capability: String,
}

impl Handoff {
	fn active(&self) -> bool {
		!self.closed.get() && self.opened.elapsed() <= LIFETIME
	}

	fn accept(&self, uri: &str, body: &str) -> bool {
		if !self.active() || self.delivered.get() || uri != PAGE {
			return false;
		}
		let Some(result) = parse_result(body, &self.capability) else {
			return false;
		};
		self.result.replace(Some(result));
		self.delivered.set(true);
		true
	}

	fn close(&self) {
		self.closed.set(true);
		self.result.borrow_mut().take();
	}
}

pub struct CaptchaView {
	view: webkit6::WebView,
	window: gtk4::Window,
	manager: webkit6::UserContentManager,
	state: Rc<Handoff>,
	cancel: gio::Cancellable,
	take_script: String,
	wake: Arc<dyn Fn() + Send + Sync>,
}

impl CaptchaView {
	pub fn open(
		parent: Arc<winit::window::Window>,
		challenge: &Challenge,
		dark: bool,
		wake: impl Fn() + Send + Sync + 'static,
	) -> Result<Self, &'static str> {
		gtk4::init().map_err(|_| "Linux verification window unavailable.")?;
		let (capability, captcha_script, html) = page(challenge, dark)?;
		let script = include_str!("captcha-linux-bridge.js")
			.replace("__SEREIN_CAPTCHA_CAPABILITY__", &capability)
			+ "\n" + &captcha_script;
		// WebKit evaluates in the main frame; bound the string before copying into Rust.
		let take_script = format!(
			"(() => {{ if (window !== window.top || location.href !== '{PAGE}') return null; const take = window['__serein_captcha_take_{}']; if (typeof take !== 'function') return null; const value = take(); return typeof value === 'string' && value.length <= 8270 && /^[\\x21-\\x7e]+$/.test(value) ? value : null; }})()",
			capability.trim_end_matches(':')
		);
		let opened = Instant::now();
		let state = Rc::new(Handoff {
			opened,
			closed: Cell::new(false),
			querying: Cell::new(false),
			delivered: Cell::new(false),
			last_query: Cell::new(opened),
			result: RefCell::new(None),
			capability,
		});
		let wake: Arc<dyn Fn() + Send + Sync> = Arc::new(wake);
		let cancel = gio::Cancellable::new();
		let session = webkit6::NetworkSession::new_ephemeral();
		session.set_persistent_credential_storage_enabled(false);
		session.set_tls_errors_policy(webkit6::TLSErrorsPolicy::Fail);
		session.connect_download_started(|_, download| download.cancel());
		let settings = webkit6::Settings::new();
		// Same identity as the REST client that submits the passcode; see client_core::fingerprint.
		settings.set_user_agent(Some(&client_core::fingerprint::user_agent()));
		settings.set_enable_developer_extras(false);
		settings.set_enable_write_console_messages_to_stdout(false);
		settings.set_allow_file_access_from_file_urls(false);
		settings.set_allow_universal_access_from_file_urls(false);
		settings.set_allow_top_navigation_to_data_urls(false);
		settings.set_allow_modal_dialogs(false);
		settings.set_javascript_can_open_windows_automatically(false);
		settings.set_javascript_can_access_clipboard(false);
		settings.set_enable_media_stream(false);
		settings.set_enable_webrtc(false);
		settings.set_enable_media(false);
		settings.set_enable_webaudio(false);
		// Software rendering prevents DMA-BUF/EGL initialization crashes on Wayland and in Flatpak.
		settings.set_hardware_acceleration_policy(webkit6::HardwareAccelerationPolicy::Never);
		let manager = webkit6::UserContentManager::new();
		manager.add_script(&webkit6::UserScript::new(
			&script,
			webkit6::UserContentInjectedFrames::TopFrame,
			webkit6::UserScriptInjectionTime::Start,
			&["https://serein-captcha.verification.invalid/*"],
			&[],
		));
		let view = webkit6::WebView::builder()
			.network_session(&session)
			.user_content_manager(&manager)
			.settings(&settings)
			.build();
		view.connect_decide_policy(|_, decision, kind| {
			let allowed = match kind {
				webkit6::PolicyDecisionType::NavigationAction => decision
					.downcast_ref::<webkit6::NavigationPolicyDecision>()
					.and_then(|decision| decision.navigation_action())
					.and_then(|action| action.request())
					.and_then(|request| request.uri())
					.is_some_and(|uri| allowed_frame(&uri)),
				webkit6::PolicyDecisionType::Response => decision
					.downcast_ref::<webkit6::ResponsePolicyDecision>()
					.is_some_and(|response| {
						response.is_mime_type_supported()
							&& (!response.is_main_frame_main_resource()
								|| response
									.request()
									.and_then(|request| request.uri())
									.is_some_and(|uri| uri == PAGE))
					}),
				_ => false,
			};
			if allowed {
				decision.use_();
			} else {
				decision.ignore();
			}
			true
		});
		view.connect_create(|_, _| None);
		view.connect_permission_request(|_, request| {
			request.deny();
			true
		});
		view.connect_query_permission_state(|_, query| {
			query.finish(webkit6::PermissionState::Denied);
			true
		});
		view.connect_run_file_chooser(|_, request| {
			request.cancel();
			true
		});
		view.connect_authenticate(|_, request| {
			request.cancel();
			true
		});
		view.connect_context_menu(|_, _, _| true);
		view.connect_enter_fullscreen(|_| true);
		view.connect_print(|_, _| true);
		view.connect_show_notification(|_, _| true);
		let window = gtk4::Window::builder()
			.title("Verification · Serein")
			.default_width(500)
			.default_height(560)
			.child(&view)
			.build();
		let weak_window = window.downgrade();
		view.connect_close(move |_| {
			if let Some(window) = weak_window.upgrade() {
				window.close();
			}
		});
		let weak_state = Rc::downgrade(&state);
		let weak_view = view.downgrade();
		let close_cancel = cancel.clone();
		let notify = wake.clone();
		window.connect_close_request(move |_| {
			if let Some(state) = weak_state.upgrade() {
				state.close();
			}
			close_cancel.cancel();
			if let Some(view) = weak_view.upgrade() {
				view.stop_loading();
				view.terminate_web_process();
			}
			notify();
			glib::Propagation::Proceed
		});
		let weak_state = Rc::downgrade(&state);
		let notify = wake.clone();
		view.connect_web_process_terminated(move |_, _| {
			if let Some(state) = weak_state.upgrade() {
				state.close();
			}
			notify();
		});
		// GTK owns its standalone window; no foreign winit/raw-handle embedding.
		let _ = parent;
		view.load_html(&html, Some(PAGE));
		window.present();
		Ok(Self {
			view,
			window,
			manager,
			state,
			cancel,
			take_script,
			wake,
		})
	}

	pub fn poll(&self) -> Option<Result<Solution, &'static str>> {
		self.pump();
		if !self.state.active() {
			self.state.close();
			return None;
		}
		self.state.result.borrow_mut().take()
	}

	pub fn expired(&self) -> bool {
		!self.state.active()
	}

	pub fn set_bounds(&self, _x: i32, _y: i32, _width: u32, _height: u32) {}

	fn pump(&self) {
		let context = glib::MainContext::default();
		let started = Instant::now();
		for _ in 0..16 {
			if started.elapsed() >= Duration::from_millis(2) || !context.pending() {
				break;
			}
			context.iteration(false);
		}
		if !self.state.active()
			|| self.state.delivered.get()
			|| self.state.querying.get()
			|| self.state.last_query.get().elapsed() < QUERY_INTERVAL
			|| !self.view.uri().is_some_and(|uri| uri == PAGE)
		{
			return;
		}
		self.state.querying.set(true);
		self.state.last_query.set(Instant::now());
		let weak_state = Rc::downgrade(&self.state);
		let weak_view = self.view.downgrade();
		let notify = self.wake.clone();
		self.view.evaluate_javascript(
			&self.take_script,
			None,
			None,
			Some(&self.cancel),
			move |result| {
				let (Some(state), Some(view)) = (weak_state.upgrade(), weak_view.upgrade()) else {
					return;
				};
				state.querying.set(false);
				if !state.active() {
					return;
				}
				if let Ok(value) = result
					&& value.is_string()
					&& let Some(uri) = view.uri()
					&& uri == PAGE
				{
					let body: zeroize::Zeroizing<String> =
						zeroize::Zeroizing::new(value.to_str().into());
					if state.accept(&uri, &body) {
						notify();
					}
				}
			},
		);
	}
}

impl Drop for CaptchaView {
	fn drop(&mut self) {
		self.state.close();
		self.cancel.cancel();
		self.manager.remove_all_scripts();
		self.view.stop_loading();
		self.view.terminate_web_process();
		self.window.set_child(None::<&gtk4::Widget>);
		self.window.destroy();
	}
}

fn allowed_frame(value: &str) -> bool {
	value == PAGE || super::hcaptcha_origin(value)
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn handoff_requires_local_page_capability_and_live_single_use_session() {
		let state = Handoff {
			opened: Instant::now(),
			closed: Cell::new(false),
			querying: Cell::new(false),
			delivered: Cell::new(false),
			last_query: Cell::new(Instant::now()),
			result: RefCell::new(None),
			capability: "a".repeat(64) + ":",
		};
		let body = state.capability.clone() + "verified:synthetic-passcode";
		assert!(!state.accept("https://hcaptcha.com/", &body));
		assert!(!state.accept(PAGE, "wrong:verified:synthetic-passcode"));
		assert!(!state.accept(
			PAGE,
			&(state.capability.clone() + "verified:" + &"x".repeat(8193))
		));
		assert!(state.accept(PAGE, &body));
		assert!(!state.accept(PAGE, &body));
		state.close();
		assert!(state.result.borrow().is_none());
		state.delivered.set(false);
		assert!(!state.accept(PAGE, &body));
		assert!(allowed_frame("https://newassets.hcaptcha.com/captcha"));
		assert!(!allowed_frame("https://hcaptcha.com.evil.test/"));
		assert!(!allowed_frame("http://hcaptcha.com/"));
	}
}
