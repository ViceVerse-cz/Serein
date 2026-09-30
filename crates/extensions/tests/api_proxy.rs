use extensions::{
	Action, ApiProxyConfig, Capability, Error, ExtensionKind, Invocation, Manifest, Output, Surface,
};

fn manifest() -> Manifest {
	Manifest {
		api_version: 1,
		id: "api-proxy".into(),
		name: "API proxy".into(),
		version: "1".into(),
		author: "test".into(),
		license: "MIT".into(),
		source: "https://example.org/source".into(),
		kind: ExtensionKind::Plugin,
		capabilities: vec![Capability::ApiProxy, Capability::Storage],
		actions: vec![Action {
			id: "apply".into(),
			label: "Apply".into(),
			surface: Surface::Panel,
		}],
	}
}
#[test]
fn proxy_origin_and_capability_boundaries() {
	for url in ["http://127.0.0.1:8080", "https://[::1]:8080/"] {
		assert!(ApiProxyConfig::Url { url: url.into() }.validate().is_ok());
	}
	for url in [
		"socks5://localhost:1080",
		"http://user:secret@localhost",
		"http://localhost/path",
		"http://localhost?secret=value",
		"http://localhost/#fragment",
		"http://localhost\n",
		"http://localhost/extra/..",
		"http://@localhost",
		"http://local host",
		"http://localhost\\extra",
	] {
		assert!(ApiProxyConfig::Url { url: url.into() }.validate().is_err());
	}
	let mut manifest = manifest();
	assert!(manifest.validate().is_ok());
	let output = Output {
		api_proxy: Some(ApiProxyConfig::Automatic),
		..Default::default()
	};
	let input = Invocation {
		action: "apply".into(),
		..Default::default()
	};
	assert!(output.validate(&manifest, &input).is_ok());
	manifest.capabilities.push(Capability::AppContext);
	assert!(matches!(manifest.validate(), Err(Error::Capability)));
	manifest.capabilities = vec![Capability::Storage];
	assert!(matches!(
		output.validate(&manifest, &input),
		Err(Error::Capability)
	));
	let encoded = serde_json::to_value(serein_extension_sdk::ApiProxyConfig::Url {
		url: "http://localhost:8080".into(),
	})
	.unwrap();
	assert_eq!(
		serde_json::from_value::<ApiProxyConfig>(encoded).unwrap(),
		ApiProxyConfig::Url {
			url: "http://localhost:8080".into()
		}
	);
}

#[test]
fn proxy_modes_reject_unknown_fields_in_host_and_sdk() {
	for text in [
		r#"{"mode":"direct","password":"secret"}"#,
		r#"{"mode":"automatic","url":"http://localhost"}"#,
	] {
		assert!(serde_json::from_str::<ApiProxyConfig>(text).is_err());
		assert!(serde_json::from_str::<serein_extension_sdk::ApiProxyConfig>(text).is_err());
	}
}

#[test]
fn sdk_proxy_wrapper_flattens_into_host_output() {
	let output = serein_extension_sdk::ApiProxyOutput {
		api_proxy: Some(serein_extension_sdk::ApiProxyConfig::Automatic),
		output: serein_extension_sdk::Output {
			panel: vec![serein_extension_sdk::Element::Text {
				text: "Settings".into(),
			}],
			..Default::default()
		},
	};
	let host: Output = serde_json::from_value(serde_json::to_value(output).unwrap()).unwrap();
	assert_eq!(host.api_proxy, Some(ApiProxyConfig::Automatic));
	assert_eq!(host.panel.len(), 1);
	host.validate(
		&manifest(),
		&Invocation {
			action: "apply".into(),
			..Default::default()
		},
	)
	.unwrap();
}
