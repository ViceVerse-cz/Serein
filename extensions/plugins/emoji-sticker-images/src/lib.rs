use serein_extension_sdk::{Invocation, Output};

// The host stages an ordinary attachment; only the user can send it.
fn activate(input: Invocation) -> Output {
	Output {
		image_sharing: input.action == "activate",
		..Default::default()
	}
}
serein_extension_sdk::export!(activate);
