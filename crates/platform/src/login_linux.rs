//! A separate ephemeral GTK4/WebKit6 window for owner-operated Discord login.
use super::{Failure, SessionSecret, captcha::hcaptcha_origin, discord_origin};
use std::{
	cell::{Cell, RefCell},
	rc::Rc,
	sync::Arc,
	time::{Duration, Instant},
};
use webkit6::{gio, glib, prelude::*};

const LIFETIME: Duration = Duration::from_secs(600);
const QUERY_INTERVAL: Duration = Duration::from_millis(100);
const HANDLER: &str = "sereinLogin";

struct Handoff {
	opened: Instant,
	closed: Cell<bool>,
	crashed: Cell<bool>,
	pending: Cell<bool>,
	querying: Cell<bool>,
	delivered: Cell<bool>,
	last_query: Cell<Instant>,
	token: RefCell<Option<SessionSecret>>,
	capability: String,
}

impl Handoff {
	fn active(&self) -> bool {
		!self.closed.get() && self.opened.elapsed() <= LIFETIME
	}

	fn accept(&self, uri: &str, body: &str) -> bool {
		if !self.active() || self.delivered.get() || !discord_origin(uri) || body.len() > 2113 {
			return false;
		}
		let Some(value) = body.strip_prefix(&self.capability) else {
			return false;
		};
		let Ok(secret) = SessionSecret::from_owner_input(value.to_owned()) else {
			return false;
		};
		self.token.replace(Some(secret));
		self.delivered.set(true);
		self.pending.set(false);
		true
	}

	fn accept_authorization(&self, uri: &str, value: &str) -> bool {
		if value.len() > 2048 {
			return false;
		}
		let body = zeroize::Zeroizing::new(format!("{}{}", self.capability, value));
		self.accept(uri, &body)
	}

	fn close(&self) {
		self.closed.set(true);
		self.pending.set(false);
		self.token.borrow_mut().take();
	}
}

pub struct LoginView {
	view: webkit6::WebView,
	window: gtk4::Window,
	manager: webkit6::UserContentManager,
	state: Rc<Handoff>,
	cancel: gio::Cancellable,
	take_script: String,
	wake: Arc<dyn Fn() + Send + Sync>,
}

impl LoginView {
	pub fn open(
		parent: Arc<winit::window::Window>,
		wake: impl Fn() + Send + Sync + 'static,
	) -> Result<Self, Failure> {
		gtk4::init().map_err(|_| Failure::ProtocolAt("Linux login window unavailable"))?;
		let mut random = [0_u8; 32];
		getrandom::fill(&mut random).map_err(|_| Failure::Protocol)?;
		let capability = random
			.iter()
			.map(|byte| format!("{byte:02x}"))
			.collect::<String>()
			+ ":";
		let script = [
			include_str!("login-linux-bridge.js"),
			include_str!("login-handoff.js"),
		]
		.join("\n")
		.replace("__SEREIN_LOGIN_CAPABILITY__", &capability);
		// evaluate_javascript runs in the main frame. Restrict its result before it
		// crosses into Rust: arbitrary child-frame IPC never supplies a token body.
		let take_script = format!(
			"(() => {{ if (window !== window.top || location.origin !== 'https://discord.com') return null; const take = window['__serein_login_take_{}']; if (typeof take !== 'function') return null; const value = take(); return typeof value === 'string' && value.length <= 2113 && /^[\\x21-\\x7e]+$/.test(value) ? value : null; }})()",
			capability.trim_end_matches(':')
		);
		let opened = Instant::now();
		let state = Rc::new(Handoff {
			opened,
			closed: Cell::new(false),
			crashed: Cell::new(false),
			pending: Cell::new(false),
			querying: Cell::new(false),
			delivered: Cell::new(false),
			last_query: Cell::new(opened),
			token: RefCell::new(None),
			capability,
		});
		let wake: Arc<dyn Fn() + Send + Sync> = Arc::new(wake);
		let cancel = gio::Cancellable::new();
		let session = webkit6::NetworkSession::new_ephemeral();
		session.set_persistent_credential_storage_enabled(false);
		session.set_tls_errors_policy(webkit6::TLSErrorsPolicy::Fail);
		session.connect_download_started(|_, download| download.cancel());
		let settings = webkit6::Settings::new();
		// WebKitGTK's default identity is "Safari on Linux", which no real browser presents;
		// hCaptcha and Discord score it as automation and reject the solved login. Present the
		// same browser identity as the REST client that will use the session afterwards.
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
		// The login page needs no audio/video. A missing GStreamer sink (autoaudiosink)
		// otherwise crashes the web process as soon as Discord's app initialises audio.
		settings.set_enable_media(false);
		settings.set_enable_webaudio(false);
		// Software rendering prevents DMA-BUF/EGL initialization crashes on Wayland and in Flatpak.
		settings.set_hardware_acceleration_policy(webkit6::HardwareAccelerationPolicy::Never);
		let manager = webkit6::UserContentManager::new();
		manager.add_script(&webkit6::UserScript::new(
			&script,
			webkit6::UserContentInjectedFrames::TopFrame,
			webkit6::UserScriptInjectionTime::Start,
			&["https://discord.com/*"],
			&[],
		));
		let view = webkit6::WebView::builder()
			.network_session(&session)
			.user_content_manager(&manager)
			.settings(&settings)
			.build();
		view.set_hexpand(true);
		view.set_vexpand(true);
		let resource_state = Rc::downgrade(&state);
		let resource_view = view.downgrade();
		let resource_notify = wake.clone();
		view.connect_resource_load_started(move |_, _, request| {
			let (Some(state), Some(view)) = (resource_state.upgrade(), resource_view.upgrade())
			else {
				return;
			};
			let Some(uri) = request.uri() else { return };
			if !discord_api_uri(&uri) || !view.uri().is_some_and(|uri| discord_origin(&uri)) {
				return;
			}
			let Some(headers) = request.http_headers() else {
				return;
			};
			let Some(value) = headers.one("authorization") else {
				return;
			};
			if value.len() > 2113 {
				return;
			}
			if state.accept_authorization(&uri, &value) {
				resource_notify();
			}
		});
		let weak_state = Rc::downgrade(&state);
		let weak_view = view.downgrade();
		let notify = wake.clone();
		manager.connect_script_message_received(Some(HANDLER), move |_, value| {
			let (Some(state), Some(view)) = (weak_state.upgrade(), weak_view.upgrade()) else {
				return;
			};
			if state.active()
				&& !state.delivered.get()
				&& value.is_boolean()
				&& value.to_boolean()
				&& view.uri().is_some_and(|uri| discord_origin(&uri))
				&& !state.pending.replace(true)
			{
				notify();
			}
		});
		if !manager.register_script_message_handler(HANDLER, None) {
			return Err(Failure::ProtocolAt("Linux login bridge unavailable"));
		}
		view.connect_decide_policy(|_, decision, kind| {
			let allowed = match kind {
				webkit6::PolicyDecisionType::NavigationAction => decision
					.downcast_ref::<webkit6::NavigationPolicyDecision>()
					.and_then(|decision| decision.navigation_action())
					.and_then(|action| action.request())
					.and_then(|request| request.uri())
					// NavigationAction includes child frames. The response policy below
					// still restricts the main document to Discord.
					.is_some_and(|uri| discord_origin(&uri) || hcaptcha_origin(&uri)),
				webkit6::PolicyDecisionType::Response => decision
					.downcast_ref::<webkit6::ResponsePolicyDecision>()
					.is_some_and(|response| {
						response.is_mime_type_supported()
							&& (!response.is_main_frame_main_resource()
								|| response
									.request()
									.and_then(|request| request.uri())
									.is_some_and(|uri| discord_origin(&uri)))
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
		view.connect_permission_request(|view, request| {
			// Embedded verification's cookie access is separate from device permissions.
			// This exception grants no device permissions or persistent storage.
			let verification_storage = view.uri().is_some_and(|uri| discord_origin(&uri))
				&& request
					.downcast_ref::<webkit6::WebsiteDataAccessPermissionRequest>()
					.is_some_and(|request| {
						verification_storage_domains(
							request.current_domain().as_deref(),
							request.requesting_domain().as_deref(),
						)
					});
			if verification_storage {
				request.allow();
			} else {
				request.deny();
			}
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
			.title("Discord sign-in · Serein")
			.default_width(900)
			.default_height(700)
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
				state.crashed.set(true);
				state.close();
			}
			notify();
		});
		// GTK owns its standalone window; no foreign winit/raw-handle embedding.
		let _ = parent;
		view.load_uri("https://discord.com/login");
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

	pub fn token(&self) -> Option<SessionSecret> {
		if !self.state.active() {
			self.state.close();
			return None;
		}
		self.state.token.borrow_mut().take()
	}

	pub fn expired(&self) -> bool {
		!self.state.active()
	}

	/// The WebKit web process ended on its own (crash or kill) rather than by timeout or close.
	pub fn crashed(&self) -> bool {
		self.state.crashed.get()
	}

	pub fn resize(&self, _parent: &winit::window::Window) {}

	pub fn pump(&self) {
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
			|| !self.state.pending.get()
			|| self.state.querying.get()
			|| self.state.last_query.get().elapsed() < QUERY_INTERVAL
			|| !self.view.uri().is_some_and(|uri| discord_origin(&uri))
		{
			return;
		}
		self.state.pending.set(false);
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
					&& discord_origin(&uri)
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

impl Drop for LoginView {
	fn drop(&mut self) {
		self.state.close();
		self.cancel.cancel();
		self.manager
			.unregister_script_message_handler(HANDLER, None);
		self.manager.remove_all_scripts();
		self.view.stop_loading();
		self.view.terminate_web_process();
		self.window.set_child(None::<&gtk4::Widget>);
		self.window.destroy();
	}
}

fn verification_storage_domains(current: Option<&str>, requesting: Option<&str>) -> bool {
	current == Some("discord.com")
		&& requesting
			.is_some_and(|domain| domain == "hcaptcha.com" || domain.ends_with(".hcaptcha.com"))
}

fn discord_api_uri(value: &str) -> bool {
	let Ok(url) = url::Url::parse(value) else {
		return false;
	};
	if !discord_origin(value) {
		return false;
	}
	let mut segments = url.path_segments().into_iter().flatten();
	segments.next() == Some("api")
		&& segments.next().is_some_and(|version| {
			let Some(version) = version.strip_prefix('v') else {
				return false;
			};
			!version.is_empty() && version.bytes().all(|byte| byte.is_ascii_digit())
		}) && segments.next().is_some()
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn verification_storage_is_limited_to_hcaptcha_embedded_in_discord() {
		for domain in ["hcaptcha.com", "newassets.hcaptcha.com"] {
			assert!(verification_storage_domains(
				Some("discord.com"),
				Some(domain)
			));
		}
		for domain in [
			None,
			Some("evil.test"),
			Some("hcaptcha.com.evil.test"),
			Some("evilhcaptcha.com"),
		] {
			assert!(!verification_storage_domains(Some("discord.com"), domain));
		}
		for domain in [None, Some("evil.test"), Some("discord.com.evil.test")] {
			assert!(!verification_storage_domains(domain, Some("hcaptcha.com")));
		}
	}

	#[test]
	fn discord_api_uri_requires_the_exact_https_api_origin() {
		for uri in [
			"https://discord.com/api/v9/users/@me",
			"https://discord.com/api/v10/science?x=1",
		] {
			assert!(discord_api_uri(uri));
		}
		for uri in [
			"https://discord.com/login",
			"https://discord.com/api/x/endpoint",
			"https://discord.com/api/vx/endpoint",
			"https://discord.com/api/v9",
			"http://discord.com/api/v9/users/@me",
			"https://discord.com.evil.test/api/v9/users/@me",
			"https://discord.com:444/api/v9/users/@me",
		] {
			assert!(!discord_api_uri(uri));
		}
	}

	#[test]
	fn handoff_is_scoped_bounded_single_use_and_closed_before_late_results() {
		let opened = Instant::now();
		let mut state = Handoff {
			opened,
			closed: Cell::new(false),
			crashed: Cell::new(false),
			pending: Cell::new(false),
			querying: Cell::new(false),
			delivered: Cell::new(false),
			last_query: Cell::new(opened),
			token: RefCell::new(None),
			capability: "a".repeat(64) + ":",
		};
		let valid = state.capability.clone() + &"T".repeat(2048);
		assert_eq!(valid.len(), 2113);
		assert!(!state.accept("https://evil.test/", &valid));
		assert!(!state.accept(
			"https://discord.com/login",
			&("b".repeat(64) + ":synthetic-token-value")
		));
		assert!(!state.accept("https://discord.com/login", &(valid.clone() + "T")));
		assert!(!state.accept(
			"https://discord.com/login",
			&(state.capability.clone() + "invalid token value")
		));
		assert!(
			state.accept_authorization("https://discord.com/api/v9/users/@me", &"T".repeat(2048))
		);
		assert_eq!(state.token.borrow().as_ref().unwrap().expose().len(), 2048);
		state.token.borrow_mut().take();
		state.delivered.set(false);
		assert!(state.token.borrow().is_none());
		assert!(state.accept("https://discord.com/login", &valid));
		assert_eq!(state.token.borrow().as_ref().unwrap().expose().len(), 2048);
		state.token.borrow_mut().take();
		assert!(!state.accept("https://discord.com/login", &valid));
		state.delivered.set(false);
		state.opened = opened - LIFETIME - Duration::from_secs(1);
		assert!(!state.accept("https://discord.com/login", &valid));
		state.opened = opened;
		assert!(state.accept("https://discord.com/login", &valid));
		state.querying.set(true);
		state.pending.set(true);
		state.close();
		assert!(state.token.borrow().is_none());
		assert!(!state.pending.get());
		assert!(!state.accept("https://discord.com/login", &valid));
	}
}
