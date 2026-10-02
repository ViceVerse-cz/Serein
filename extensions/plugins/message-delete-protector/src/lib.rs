use serein_extension_sdk::{Invocation, Output};

// The host retains bounded session memory; this plugin receives no message bodies.
fn activate(input: Invocation) -> Output {
	Output {
		preserve_deleted_messages: input.action == "activate",
		..Default::default()
	}
}

serein_extension_sdk::export!(activate);
