//! Explicit REST routing. Automatic reads environment proxies, not browser PAC scripts.
use reqwest::{ClientBuilder, Proxy, Url};
use serde::{Deserialize, Serialize};

#[derive(Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
pub enum ApiProxy {
	Direct,
	Automatic,
	Url { url: String },
}

// Internally tagged unit variants ignore extra fields in serde; empty structs enforce the schema.
impl<'de> Deserialize<'de> for ApiProxy {
	fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
		#[derive(Deserialize)]
		#[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
		enum Wire {
			Direct {},
			Automatic {},
			Url { url: String },
		}
		Ok(match Wire::deserialize(deserializer)? {
			Wire::Direct {} => Self::Direct,
			Wire::Automatic {} => Self::Automatic,
			Wire::Url { url } => Self::Url { url },
		})
	}
}

fn validate_url(value: &str) -> Result<(), &'static str> {
	if value.len() > 2048
		|| value.contains('@')
		|| value
			.bytes()
			.any(|b| b.is_ascii_whitespace() || b.is_ascii_control() || b == b'\\')
	{
		return Err("API proxy URL rejected");
	}
	// Check the raw suffix too: URL normalization could turn /a/.. into an allowed root.
	let (_, address) = value.split_once("://").ok_or("API proxy URL rejected")?;
	if let Some(suffix) = address.find(['/', '?', '#'])
		&& &address[suffix..] != "/"
	{
		return Err("Use an HTTP(S) proxy address without credentials, path, query or fragment");
	}
	let url = Url::parse(value).map_err(|_| "API proxy URL rejected")?;
	if !matches!(url.scheme(), "http" | "https")
		|| url.host_str().is_none()
		|| !url.username().is_empty()
		|| url.password().is_some()
		|| url.path() != "/"
		|| url.query().is_some()
		|| url.fragment().is_some()
	{
		return Err("Use an HTTP(S) proxy address without credentials, path, query or fragment");
	}
	Ok(())
}

impl ApiProxy {
	pub fn validate(&self) -> Result<(), &'static str> {
		match self {
			Self::Direct => Ok(()),
			Self::Url { url } => validate_url(url),
			Self::Automatic => {
				// Reject credentials even in an unused lowercase override; never echo environment values.
				for name in [
					"HTTP_PROXY",
					"http_proxy",
					"HTTPS_PROXY",
					"https_proxy",
					"ALL_PROXY",
					"all_proxy",
				] {
					match std::env::var(name) {
						Ok(value) if !value.is_empty() => validate_url(&value)?,
						Err(std::env::VarError::NotUnicode(_)) => {
							return Err("API proxy environment setting rejected");
						}
						_ => {}
					}
				}
				Ok(())
			}
		}
	}

	/// Explicit proxies have no direct fallback. Automatic retains reqwest's NO_PROXY handling.
	pub fn client_builder(&self, builder: ClientBuilder) -> Result<ClientBuilder, &'static str> {
		self.validate()?;
		Ok(match self {
			Self::Direct => builder.no_proxy(),
			Self::Automatic => builder,
			Self::Url { url } => builder
				.no_proxy()
				.proxy(Proxy::all(url).map_err(|_| "API proxy URL rejected")?),
		})
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn accepts_only_bounded_credential_free_http_proxy_addresses() {
		for url in [
			"http://127.0.0.1:8080",
			"https://proxy.example/",
			"http://[::1]:8080/",
		] {
			assert!(ApiProxy::Url { url: url.into() }.validate().is_ok());
		}
		for url in [
			"",
			"socks5://proxy.example",
			"http://user:secret@proxy.example",
			"http://@proxy.example",
			"http://proxy.example/path",
			"http://proxy.example/a/..",
			"http://proxy.example/./",
			"http://proxy.example?token=secret",
			"http://proxy.example#fragment",
			"http://proxy.example\\evil",
			"http://proxy.example\n",
		] {
			assert!(ApiProxy::Url { url: url.into() }.validate().is_err());
		}
		assert!(
			ApiProxy::Url {
				url: format!("http://{}/", "x".repeat(2048))
			}
			.validate()
			.is_err()
		);
		let route: ApiProxy =
			serde_json::from_str(r#"{"mode":"url","url":"http://proxy.example:8080"}"#).unwrap();
		assert!(route.validate().is_ok());
		for value in [
			r#"{"mode":"direct","password":"secret"}"#,
			r#"{"mode":"automatic","url":"http://proxy.example"}"#,
			r#"{"mode":"url","url":"http://proxy.example","password":"secret"}"#,
		] {
			assert!(serde_json::from_str::<ApiProxy>(value).is_err());
		}
		assert!(matches!(
			serde_json::from_str::<ApiProxy>(r#"{"mode":"direct"}"#).unwrap(),
			ApiProxy::Direct
		));
		assert!(matches!(
			serde_json::from_str::<ApiProxy>(r#"{"mode":"automatic"}"#).unwrap(),
			ApiProxy::Automatic
		));
	}
}
