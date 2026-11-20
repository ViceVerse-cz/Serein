//! Explicit offline Safety Hub fixture. Reads no saved session and contacts no service.
use client_core::Event;
use test_support::safety_hub::Fixture;

/// The fixture selected by `SEREIN_DEMO_STANDING` or `--demo-standing=<name>`; default good.
fn selected() -> Fixture {
	std::env::var("SEREIN_DEMO_STANDING")
		.ok()
		.as_deref()
		.and_then(Fixture::parse)
		.or_else(|| {
			std::env::args().find_map(|arg| {
				arg.strip_prefix("--demo-standing=")
					.and_then(Fixture::parse)
			})
		})
		.unwrap_or(Fixture::Good)
}

pub fn execute(request: u64) -> Event {
	Event::SafetyHub {
		request,
		result: Ok(selected().snapshot()),
	}
}
