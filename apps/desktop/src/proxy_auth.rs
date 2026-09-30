//! Resolves host-owned proxy credentials off rendering; secrets never enter Wasm.
use discord_api::proxy::ApiProxy;
use platform::proxy_credentials::Credentials;
use std::sync::mpsc;

type Result = std::result::Result<Option<Credentials>, &'static str>;
#[derive(Default)]
pub struct Authentication {
	endpoint: Option<String>,
	route: Option<ApiProxy>,
	pending: Option<(String, mpsc::Receiver<Result>)>,
}
fn endpoint(value: &str) -> std::result::Result<String, &'static str> {
	ApiProxy::Url { url: value.into() }.validate()?;
	url::Url::parse(value)
		.map(|url| url.to_string())
		.map_err(|_| "Invalid proxy URL")
}
impl Authentication {
	fn start(
		&mut self,
		endpoint: String,
		runtime: &tokio::runtime::Runtime,
		ctx: &eframe::egui::Context,
		work: impl FnOnce() -> Result + Send + 'static,
	) {
		let (send, receive) = mpsc::sync_channel(1);
		self.pending = Some((endpoint, receive));
		let ctx = ctx.clone();
		runtime.spawn_blocking(move || {
			let _ = send.send(work());
			ctx.request_repaint();
		});
	}
	pub fn tick(
		&mut self,
		config: extensions::ApiProxyConfig,
		form: &mut ui::proxy_auth::Form,
		runtime: &tokio::runtime::Runtime,
		ctx: &eframe::egui::Context,
		demo: bool,
	) -> Option<ApiProxy> {
		if let Some((url, receive)) = &self.pending {
			match receive.try_recv() {
				Ok(result) => {
					if self.endpoint.as_ref() == Some(url) {
						match result {
							Ok(credentials) => {
								self.route = Some(match credentials {
									Some(value) if value.endpoint == *url => {
										ApiProxy::Authenticated {
											url: url.clone(),
											username: std::sync::Arc::new(value.username),
											password: std::sync::Arc::new(value.password),
										}
									}
									_ => ApiProxy::Url { url: url.clone() },
								});
								form.status = "Proxy credentials loaded. Future API requests use this setting.".into();
							}
							Err(error) => {
								self.route = None;
								form.status = error.into();
							}
						}
					}
					self.pending = None;
				}
				Err(mpsc::TryRecvError::Disconnected) => {
					self.route = None;
					self.pending = None;
					form.status =
						"Proxy credential operation ended; retry saving or removing credentials."
							.into();
				}
				Err(mpsc::TryRecvError::Empty) => {}
			}
		}
		let current = match config {
			extensions::ApiProxyConfig::Direct => {
				self.endpoint = None;
				self.route = None;
				Some(ApiProxy::Direct)
			}
			extensions::ApiProxyConfig::Automatic => {
				self.endpoint = None;
				self.route = None;
				Some(ApiProxy::Automatic)
			}
			extensions::ApiProxyConfig::Url { url } => {
				let Ok(url) = endpoint(&url) else {
					form.status = "Invalid proxy URL".into();
					return None;
				};
				if self.endpoint.as_ref() != Some(&url) {
					self.route = None;
				}
				if self.endpoint.as_ref() != Some(&url) && self.pending.is_none() {
					self.endpoint = Some(url.clone());
					self.route = None;
					self.start(url, runtime, ctx, move || {
                        if demo { return Ok(None); }
                        match platform::proxy_credentials::load() {
                            Ok(value) => Ok(value),
                            Err(platform::CredentialError::NoStore) => Ok(None),
                            Err(_) => Err("OS credential store unavailable. API routing is paused; retry saving or removing credentials."),
                        }
                    });
				}
				self.route.clone()
			}
		};
		if let Some(request) = form.request.take() {
			if demo {
				form.status = "Demo mode does not read or save proxy credentials.".into();
			} else if self.pending.is_some() {
				form.status = "Wait for the current credential operation.".into();
			} else if let Ok(url) = endpoint(&request.endpoint) {
				if self.endpoint.as_ref() != Some(&url) {
					form.status = "Apply this URL in the plugin before saving credentials.".into();
				} else {
					let value = request.credentials.map(|(username, password)| Credentials {
						endpoint: url.clone(),
						username,
						password,
					});
					if value
						.as_ref()
						.is_some_and(|value| value.validate().is_err())
					{
						form.status = "Use a username of 1-256 bytes without a colon, and a password up to 1024 bytes; control characters are unsupported.".into();
					} else {
						self.start(url, runtime, ctx, move || {
							let result = match &value {
								Some(value) => platform::proxy_credentials::save(value),
								None => platform::proxy_credentials::forget(),
							};
							result.map(|()| value).map_err(
								|_| "OS credential store update failed; stored credentials retained, API routing paused. Retry Save or Remove.",
							)
						});
					}
				}
			} else {
				form.status = "Enter a valid HTTP/HTTPS proxy URL first.".into();
			}
		}
		form.busy = self.pending.is_some();
		current
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn changed_endpoint_waits_and_direct_discards_late_credentials() {
		let runtime = tokio::runtime::Runtime::new().unwrap();
		let ctx = eframe::egui::Context::default();
		let mut form = ui::proxy_auth::Form::default();
		let (send, receive) = mpsc::sync_channel(1);
		let old = "http://old.invalid/".to_owned();
		let mut auth = Authentication {
			endpoint: Some(old.clone()),
			route: Some(ApiProxy::Url { url: old.clone() }),
			pending: Some((old, receive)),
		};
		assert!(
			auth.tick(
				extensions::ApiProxyConfig::Url {
					url: "http://new.invalid/".into()
				},
				&mut form,
				&runtime,
				&ctx,
				true
			)
			.is_none()
		);
		assert!(matches!(
			auth.tick(
				extensions::ApiProxyConfig::Direct,
				&mut form,
				&runtime,
				&ctx,
				true
			),
			Some(ApiProxy::Direct)
		));
		send.send(Ok(Some(Credentials {
			endpoint: "http://old.invalid/".into(),
			username: zeroize::Zeroizing::new("owner".into()),
			password: zeroize::Zeroizing::new("synthetic".into()),
		})))
		.unwrap();
		assert!(matches!(
			auth.tick(
				extensions::ApiProxyConfig::Direct,
				&mut form,
				&runtime,
				&ctx,
				true
			),
			Some(ApiProxy::Direct)
		));
		assert!(auth.route.is_none() && auth.pending.is_none());
	}
}
