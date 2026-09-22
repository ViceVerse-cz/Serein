use serein_extension_sdk::{Invocation, Output};

fn activate(_input: Invocation) -> Output {
	Output::default()
}

serein_extension_sdk::export!(activate);
