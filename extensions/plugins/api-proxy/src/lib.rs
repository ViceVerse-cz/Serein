use serein_extension_sdk::{ApiProxyConfig, ApiProxyOutput, Element, Invocation};

fn panel(mode: &str, url: &str, notice: &str) -> Vec<Element> {
	let mut elements = vec![
		Element::Heading { text: "API Proxy".into() },
		Element::Text { text: "Only Discord REST API requests use this setting. Gateway, CDN/media and voice calls remain direct.".into() },
		Element::Select {
			id: "mode".into(), label: "Connection mode".into(),
			options: vec!["Direct".into(), "Automatic".into(), "URL".into()], value: mode.into(),
		},
		Element::TextInput { id: "url".into(), label: "HTTP/HTTPS proxy URL".into(), value: url.into() },
		Element::Text { text: "Automatic uses environment proxy settings; PAC scripts are unsupported. A URL may contain only a host and optional port. Draft edits take effect only after Apply. For authenticated proxies, use Serein's Proxy authentication section below; never paste credentials into the URL.".into() },
		Element::Button { id: "apply".into(), label: "Apply API proxy".into() },
	];
	if !notice.is_empty() {
		elements.push(Element::Text {
			text: notice.into(),
		});
	}
	elements
}

fn handle(input: Invocation) -> ApiProxyOutput {
	let stored = input.storage_json::<ApiProxyConfig>();
	let corrupt = stored.is_err()
		|| stored
			.as_ref()
			.ok()
			.and_then(|value| value.as_ref())
			.is_some_and(|value| value.validate().is_err());
	let saved = stored
		.ok()
		.flatten()
		.filter(|value| value.validate().is_ok())
		.unwrap_or(ApiProxyConfig::Direct);
	if input.action == "activate" {
		assert!(
			!corrupt,
			"Saved API proxy settings are invalid; disable and re-enable the plugin."
		);
		return ApiProxyOutput {
			api_proxy: Some(saved),
			..Default::default()
		};
	}
	if !matches!(input.action.as_str(), "open" | "apply") {
		return ApiProxyOutput::default();
	}
	let (saved_mode, saved_url) = match &saved {
		ApiProxyConfig::Direct => ("Direct", ""),
		ApiProxyConfig::Automatic => ("Automatic", ""),
		ApiProxyConfig::Url { url } => ("URL", url.as_str()),
	};
	let mode = if input.action == "apply" {
		input.value("mode").unwrap_or(saved_mode)
	} else {
		saved_mode
	};
	// Panel inputs are host-bounded to 4 KiB; configuration has a 2 KiB URL ceiling.
	let url = if input.action == "apply" {
		input.value("url").unwrap_or(saved_url)
	} else {
		saved_url
	};
	let url = url.trim();
	let valid_mode = ["Direct", "Automatic", "URL"].contains(&mode);
	let mode = if valid_mode { mode } else { "Direct" };
	let mut notice = if corrupt {
		"Saved settings were invalid. Direct mode is selected; review and Apply to replace them."
	} else {
		""
	};
	let mut output = ApiProxyOutput::default();
	if input.action == "apply" && !valid_mode {
		notice = "Choose Direct, Automatic or URL mode.";
	} else if input.action == "apply" {
		let config = match mode {
			"Automatic" => ApiProxyConfig::Automatic,
			"URL" => ApiProxyConfig::Url { url: url.into() },
			_ => ApiProxyConfig::Direct,
		};
		match config.validate() {
			Err(error) => notice = error,
			Ok(()) => {
				if output.output.set_storage_json(&config).is_ok() {
					output.api_proxy = Some(config);
					notice = "Saved. The API connection setting has been submitted.";
				} else {
					notice = "Could not save these settings. Nothing was applied.";
				}
			}
		}
	}
	// Never echo oversized input or a credential-bearing draft into another response.
	let shown_url = if url.len() <= 2048 && !url.contains('@') {
		url
	} else {
		""
	};
	output.output.panel = panel(mode, shown_url, notice);
	output
}

serein_extension_sdk::export!(handle);

#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn apply_persists_but_open_is_passive_and_activation_restores() {
		let mut input = Invocation {
			action: "apply".into(),
			..Default::default()
		};
		input.values.insert("mode".into(), "URL".into());
		input
			.values
			.insert("url".into(), "http://127.0.0.1:8080".into());
		let applied = handle(input.clone());
		assert!(matches!(
			applied.api_proxy,
			Some(ApiProxyConfig::Url { .. })
		));
		input.storage = applied.output.storage;
		input.values.insert("mode".into(), "Direct".into());
		input.action = "open".into();
		let opened = handle(input.clone());
		assert!(opened.api_proxy.is_none() && opened.output.storage.is_none());
		assert!(
			opened
				.output
				.panel
				.iter()
				.any(|element| matches!(element, Element::Select { value, .. } if value == "URL"))
		);
		input.action = "activate".into();
		assert!(matches!(
			handle(input).api_proxy,
			Some(ApiProxyConfig::Url { .. })
		));
	}
	#[test]
	fn rejected_urls_are_never_stored_or_applied() {
		for url in [
			"http://user:secret@example.com",
			"https://example.com/path",
			"https://example.com?secret=x",
			"socks5://localhost:8080",
			&"x".repeat(4096),
		] {
			let mut input = Invocation {
				action: "apply".into(),
				..Default::default()
			};
			input.values.insert("mode".into(), "URL".into());
			input.values.insert("url".into(), url.into());
			let output = handle(input);
			assert!(
				output.api_proxy.is_none() && output.output.storage.is_none(),
				"{url}"
			);
			assert!(
				serein_extension_sdk::serde_json::to_vec(&output)
					.unwrap()
					.len() < 8192
			);
		}
		for storage in [
			"{invalid",
			r#"{"mode":"url","url":"http://user:secret@example.com"}"#,
		] {
			let input = Invocation {
				action: "activate".into(),
				storage: Some(storage.into()),
				..Default::default()
			};
			assert!(std::panic::catch_unwind(|| handle(input)).is_err());
		}
		for (mode, expected) in [
			("Direct", ApiProxyConfig::Direct),
			("Automatic", ApiProxyConfig::Automatic),
		] {
			let mut input = Invocation {
				action: "apply".into(),
				..Default::default()
			};
			input.values.insert("mode".into(), mode.into());
			let output = handle(input);
			assert_eq!(output.api_proxy, Some(expected));
			assert!(output.output.storage.is_some());
		}
	}
}
