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
		// Expire a known track even when the next network request stalls.
		let deadline = remaining(&activity).min(Duration::from_secs(30));
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
		let delay = remaining(&activity).min(Duration::from_secs(15));
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
