//! Latest-value custom presence; resolution never runs on the rendering thread.
use super::*;
use extensions::{CustomRichPresence, RichPresenceKind, RichPresenceTimer};

pub(super) async fn run<A: Applications>(
	mut requests: watch::Receiver<Option<CustomRichPresence>>,
	output: watch::Sender<Result<Option<Activity>, &'static str>>,
	service: A,
) {
	let mut session = None;
	let mut elapsed_start = None;
	loop {
		let request = requests.borrow_and_update().clone();
		if request.is_none() {
			elapsed_start = None;
			session = None;
		}
		let result = async {
			let Some(request) = request.as_ref() else {
				return Ok(None);
			};
			request
				.validate()
				.map_err(|_| "Custom activity contains invalid fields.")?;
			let application = request.application_id.parse::<Id>().map_err(|_| INVALID)?;
			if session
				.as_ref()
				.is_none_or(|s: &Session<'_, A>| s.application != application)
			{
				session = Some(Session::new(application, &service));
			}
			let now = time::OffsetDateTime::now_utc();
			let timestamps = timestamps(&request.timer, &mut elapsed_start, now)?;
			let mut activity = Activity {
				name: request.name.clone(),
				application_id: application,
				kind: match request.kind {
					RichPresenceKind::Playing => 0,
					RichPresenceKind::Streaming => 1,
					RichPresenceKind::Listening => 2,
					RichPresenceKind::Watching => 3,
					RichPresenceKind::Competing => 5,
				},
				details: request.details.clone(),
				state: request.state.clone(),
				timestamps,
				assets: (request.large_image.is_some() || request.small_image.is_some()).then(
					|| rpc::Assets {
						large_image: request.large_image.as_ref().map(|i| i.key.clone()),
						large_text: request.large_image.as_ref().and_then(|i| i.text.clone()),
						large_url: request.large_image.as_ref().and_then(|i| i.url.clone()),
						small_image: request.small_image.as_ref().map(|i| i.key.clone()),
						small_text: request.small_image.as_ref().and_then(|i| i.text.clone()),
						small_url: request.small_image.as_ref().and_then(|i| i.url.clone()),
					},
				),
				extra: rpc::ActivityExtra {
					url: request.stream_url.clone(),
					details_url: request.details_url.clone(),
					state_url: request.state_url.clone(),
					buttons: request.buttons.iter().map(|b| b.label.clone()).collect(),
					metadata: (!request.buttons.is_empty()).then(|| rpc::ButtonMetadata {
						button_urls: request.buttons.iter().map(|b| b.url.clone()).collect(),
					}),
					party: request.party.as_ref().map(|p| rpc::Party {
						size: [p.current, p.max],
					}),
				},
			};
			activity
				.validate()
				.map_err(|_| "Custom activity is too large. Shorten its text or links.")?;
			if let Some(assets) = activity.assets.take() {
				let session = session.as_mut().expect("custom session");
				if session.metadata.is_none() {
					session.metadata = Some(service.metadata(application).await?);
				}
				let resolved = session.resolve(assets.clone()).await;
				if assets.large_image.is_some() && resolved.large_image.is_none()
					|| assets.small_image.is_some() && resolved.small_image.is_none()
				{
					return Err(
						"Custom activity artwork could not be resolved. Check the application ID and image keys or HTTPS URLs.",
					);
				}
				activity.assets = Some(resolved);
			}
			activity
				.validate()
				.map_err(|_| "Custom activity is too large. Shorten its text or links.")?;
			Ok(Some(activity))
		};
		tokio::select! {
			biased;
			changed = requests.changed() => { if changed.is_err() { return; } continue; }
			result = result => {
				output.send_if_modified(|current| {
					if *current == result { false } else { *current = result; true }
				});
			}
		}
		tokio::select! {
			changed = requests.changed() => { if changed.is_err() { return; } }
			_ = tokio::time::sleep(Duration::from_secs(60)), if request.as_ref().is_some_and(|r| r.timer == RichPresenceTimer::LocalDay) => {}
		}
	}
}

fn timestamps(
	timer: &RichPresenceTimer,
	elapsed: &mut Option<u64>,
	now: time::OffsetDateTime,
) -> Result<Option<rpc::Timestamps>, &'static str> {
	let millis = u64::try_from(now.unix_timestamp_nanos() / 1_000_000)
		.map_err(|_| "The system clock is unavailable.")?;
	Ok(match timer {
		RichPresenceTimer::None => {
			*elapsed = None;
			None
		}
		RichPresenceTimer::Elapsed => Some(rpc::Timestamps {
			start: Some(*elapsed.get_or_insert(millis)),
			end: None,
		}),
		RichPresenceTimer::Custom { start, end } => {
			*elapsed = None;
			Some(rpc::Timestamps {
				start: *start,
				end: *end,
			})
		}
		RichPresenceTimer::LocalDay => {
			*elapsed = None;
			Some(rpc::Timestamps {
				start: Some(local_midnight(now)?),
				end: None,
			})
		}
	})
}

fn local_midnight(now: time::OffsetDateTime) -> Result<u64, &'static str> {
	const ERROR: &str =
		"Local midnight could not be determined safely. Choose an elapsed or custom timer.";
	let offset = time::UtcOffset::local_offset_at(now).map_err(|_| ERROR)?;
	let midnight = now.to_offset(offset).date().midnight();
	let mut candidate = midnight.assume_offset(offset);
	// The offset at midnight can differ from today's offset across a daylight-saving change.
	for _ in 0..3 {
		let offset = time::UtcOffset::local_offset_at(candidate).map_err(|_| ERROR)?;
		if candidate.to_offset(offset).date() == midnight.date()
			&& candidate.to_offset(offset).time() == time::Time::MIDNIGHT
		{
			return u64::try_from(candidate.unix_timestamp_nanos() / 1_000_000).map_err(|_| ERROR);
		}
		candidate = midnight.assume_offset(offset);
	}
	Err(ERROR)
}

#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn elapsed_survives_updates_and_custom_is_exact_milliseconds() {
		let now = time::OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap();
		let mut elapsed = None;
		let first = timestamps(&RichPresenceTimer::Elapsed, &mut elapsed, now).unwrap();
		assert_eq!(
			timestamps(
				&RichPresenceTimer::Elapsed,
				&mut elapsed,
				now + time::Duration::seconds(20)
			)
			.unwrap(),
			first
		);
		assert_eq!(
			timestamps(
				&RichPresenceTimer::Custom {
					start: Some(42),
					end: Some(99)
				},
				&mut elapsed,
				now
			)
			.unwrap(),
			Some(rpc::Timestamps {
				start: Some(42),
				end: Some(99)
			})
		);
		assert!(elapsed.is_none());
		assert_eq!(
			timestamps(
				&RichPresenceTimer::Elapsed,
				&mut elapsed,
				now + time::Duration::seconds(20)
			)
			.unwrap()
			.unwrap()
			.start,
			Some(1_700_000_020_000)
		);
	}
}
