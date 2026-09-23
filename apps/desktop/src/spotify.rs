//! Linked-account playback, independent of local game detection or Spotify IPC.
use client_core::auth::Failure;
use discord_protocol::spotify::Activity;
use std::{sync::Arc, time::Duration};
use tokio::sync::watch;

pub async fn run(
	api: Arc<discord_api::DiscordApi>,
	user: model::Id,
	mut presence: watch::Receiver<model::OwnPresence>,
	activity: watch::Sender<Option<Activity>>,
	ctx: eframe::egui::Context,
) {
	let Ok(mut playback) = discord_api::spotify::Playback::new() else {
		return;
	};
	loop {
		if api.stopped() {
			publish(&activity, None, &ctx);
			return;
		}
		if presence.borrow_and_update().status == model::PresenceStatus::Invisible {
			publish(&activity, None, &ctx);
			playback.clear_token();
			if presence.changed().await.is_err() {
				return;
			}
			continue;
		}
		// Let the next track arrive before replacing the current activity.
		// A zero timeout at track end used to cancel the poll and clear presence.
		let deadline = poll_timeout(remaining(&activity));
		let result = tokio::select! {
			biased;
			changed = presence.changed() => {
				if changed.is_err() { publish(&activity, None, &ctx); return; }
				continue;
			}
			result = tokio::time::timeout(deadline, playback.poll(&api, user)) => {
				result.unwrap_or(Err(Failure::Network))
			}
		};
		publish(&activity, result.ok().flatten(), &ctx);
		let delay = remaining(&activity).clamp(Duration::from_secs(1), Duration::from_secs(15));
		tokio::select! {
			changed = presence.changed() => {
				if changed.is_err() { publish(&activity, None, &ctx); return; }
			}
			_ = tokio::time::sleep(delay) => {}
		}
	}
}

fn remaining(activity: &watch::Sender<Option<Activity>>) -> Duration {
	let now = std::time::SystemTime::now()
		.duration_since(std::time::UNIX_EPOCH)
		.unwrap_or_default()
		.as_millis();
	activity
		.borrow()
		.as_ref()
		.and_then(|a| a.timestamps.end)
		.map_or(Duration::from_secs(30), |end| {
			Duration::from_millis(end.saturating_sub(now.min(u64::MAX as u128) as u64))
		})
}

fn publish(
	sender: &watch::Sender<Option<Activity>>,
	next: Option<Activity>,
	ctx: &eframe::egui::Context,
) {
	if sender.send_if_modified(|current| {
		if *current == next {
			return false;
		}
		*current = next;
		true
	}) {
		ctx.request_repaint();
	}
}

fn poll_timeout(remaining: Duration) -> Duration {
	remaining.clamp(Duration::from_secs(10), Duration::from_secs(30))
}

#[cfg(all(debug_assertions, feature = "demo"))]
pub fn debug_check() {
	for remaining in [
		Duration::ZERO,
		Duration::from_millis(1),
		Duration::from_secs(5),
	] {
		assert_eq!(poll_timeout(remaining), Duration::from_secs(10));
	}
	assert_eq!(
		poll_timeout(Duration::from_secs(90)),
		Duration::from_secs(30)
	);
}
