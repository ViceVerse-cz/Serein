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

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn handoff_is_scoped_bounded_single_use_and_closed_before_late_results() {
		let opened = Instant::now();
		let mut state = Handoff {
			opened,
			closed: Cell::new(false),
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
