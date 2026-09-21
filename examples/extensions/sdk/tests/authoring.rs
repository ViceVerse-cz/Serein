use serein_extension_sdk::{Error, Invocation, MAX_IO_BYTES, Output, dispatch, serde_json};

// These names must not affect the exported macro's implementation.
#[allow(dead_code, unused_macros)]
mod abi {
	struct Box;
	mod std {}
	macro_rules! vec {
		($($tokens:tt)*) => {
			compile_error!("export! used the caller's vec! macro")
		};
	}

	fn handler(_: super::Invocation) -> super::Output {
		panic!("invalid ABI input reached handler")
	}

	serein_extension_sdk::export!(handler);
}

#[test]
fn abi_rejects_empty_null_and_oversized_buffers_before_reading_memory() {
	assert_eq!(abi::serein_alloc(0), 0);
	assert_eq!(abi::serein_alloc(MAX_IO_BYTES as u32 + 1), 0);
	for (pointer, length) in [(0, 1), (1, 0), (1, MAX_IO_BYTES as u32 + 1)] {
		// SAFETY: each invalid argument is rejected before dereferencing the pointer.
		assert_eq!(unsafe { abi::serein_invoke(pointer, length) }, 0);
	}
}

#[test]
fn native_dispatch_preserves_v1_and_reports_bad_input() {
	let response = dispatch(br#"{"action":"run","future_field":true}"#, |input| {
		assert_eq!(input.action, "run");
		assert_eq!(
			input,
			Invocation {
				action: "run".into(),
				..Default::default()
			}
		);
		Output {
			replacement: Some("hello".into()),
			..Default::default()
		}
	})
	.unwrap();
	let output: Output = serde_json::from_slice(&response).unwrap();
	assert_eq!(output.replacement.as_deref(), Some("hello"));
	let json: serde_json::Value = serde_json::from_slice(&response).unwrap();
	assert!(json.get("image_sharing").is_none());
	assert!(json.get("preserve_deleted_messages").is_none());
	assert_eq!(
		serde_json::from_str::<Output>("{}").unwrap(),
		Output::default()
	);
	for bytes in [
		b"{}".as_slice(),
		b"not JSON",
		b"{\"action\":3}",
		b"{\"action\":\"one\",\"action\":\"two\"}",
		b"{\"action\":\"one\"}{}",
		b"\xff",
	] {
		assert!(matches!(
			dispatch(bytes, |_| panic!("invalid input reached handler")),
			Err(Error::InvalidInput(_))
		));
	}
	assert!(matches!(
		dispatch(&vec![b' '; MAX_IO_BYTES + 1], |_| panic!(
			"oversized input reached handler"
		)),
		Err(Error::InputTooLarge)
	));
}

#[test]
fn byte_limits_include_json_escaping_and_accept_exact_boundary() {
	let mut input = br#"{"action":"run"}"#.to_vec();
	input.resize(MAX_IO_BYTES, b' ');
	let overhead = dispatch(&input, |_| Output {
		storage: Some(String::new()),
		..Default::default()
	})
	.unwrap()
	.len();
	let response = dispatch(&input, |_| Output {
		storage: Some("a".repeat(MAX_IO_BYTES - overhead)),
		..Default::default()
	})
	.unwrap();
	assert_eq!(response.len(), MAX_IO_BYTES);
	for storage in [
		"a".repeat(MAX_IO_BYTES - overhead + 1),
		"\0".repeat(MAX_IO_BYTES / 6),
		"🦀".repeat(MAX_IO_BYTES / 4),
	] {
		assert!(matches!(
			dispatch(&input, |_| Output {
				storage: Some(storage),
				..Default::default()
			}),
			Err(Error::OutputTooLarge)
		));
	}
}

#[test]
fn typed_values_and_storage_distinguish_missing_from_invalid() {
	let mut input = Invocation::default();
	input.values.extend([
		("checked".into(), "false".into()),
		("size".into(), "18".into()),
		("empty".into(), String::new()),
		("bad".into(), "yes".into()),
	]);
	assert_eq!(input.value("empty"), Some(""));
	assert_eq!(input.value("missing"), None);
	assert_eq!(input.parse_value::<bool>("checked"), Ok(Some(false)));
	assert_eq!(input.parse_value::<i32>("size"), Ok(Some(18)));
	assert_eq!(input.parse_value::<i32>("missing"), Ok(None));
	assert!(input.parse_value::<bool>("bad").is_err());
	assert_eq!(input.storage_json::<serde_json::Value>().unwrap(), None);
	let mut output = Output::default();
	let value = serde_json::json!({"name":"Žluťoučký", "checked": false});
	output.set_storage_json(&value).unwrap();
	input.storage = output.storage.clone();
	assert_eq!(
		input.storage_json::<serde_json::Value>().unwrap(),
		Some(value)
	);
	input.storage = Some("invalid".into());
	assert!(input.storage_json::<serde_json::Value>().is_err());
	let previous = output.clone();
	assert!(matches!(
		output.set_storage_json(&"x".repeat(MAX_IO_BYTES)),
		Err(Error::OutputTooLarge)
	));
	assert_eq!(output, previous);
	// JSON object keys cannot be arrays; serialization failures also preserve storage.
	let invalid = std::collections::BTreeMap::from([([1, 2], 3)]);
	assert!(matches!(
		output.set_storage_json(&invalid),
		Err(Error::InvalidOutput(_))
	));
	assert_eq!(output, previous);
}

#[test]
fn typed_event_dispatch_keeps_bounds_and_rejects_bad_json_before_the_handler() {
	use serein_extension_sdk::{EventInvocation, MessageEventKind, dispatch_typed};
	let bytes = dispatch_typed(
		br#"{"action":"event","message_event":{"kind":"delete","channel_id":"1","message_id":"2"}}"#,
		|input: EventInvocation| {
			assert_eq!(input.invocation.action, "event");
			let event = input.message_event.unwrap();
			assert_eq!(event.kind, MessageEventKind::Delete);
			assert!(event.author_id.is_none() && event.content.is_none());
			Output::default()
		},
	).unwrap();
	assert_eq!(
		serde_json::from_slice::<Output>(&bytes).unwrap(),
		Output::default()
	);
	for input in [
		br#"{"action":3}"#.as_slice(),
		br#"{"action":"event","message_event":{"kind":"unknown"}}"#,
	] {
		assert!(matches!(
			dispatch_typed(input, |_: EventInvocation| -> Output {
				panic!("invalid input reached handler")
			}),
			Err(Error::InvalidInput(_))
		));
	}
	assert!(matches!(
		dispatch_typed(&vec![b' '; MAX_IO_BYTES + 1], |_: EventInvocation| {
			Output::default()
		}),
		Err(Error::InputTooLarge)
	));
	assert!(matches!(
		dispatch_typed(br#"{"action":"event"}"#, |_: EventInvocation| Output {
			storage: Some("x".repeat(MAX_IO_BYTES)),
			..Default::default()
		}),
		Err(Error::OutputTooLarge)
	));
}
