use local_store::StoreError;
use model::ReadingPreferences;
use std::time::{Duration, Instant};

/// One latest value plus at most one queued write. No settings write backlog while dragging.
#[derive(Default)]
pub struct ReadingSettings {
	pub current: ReadingPreferences,
	touched: bool,
	loaded: bool,
	due: Option<Instant>,
	saving: bool,
	failed: bool,
}
impl ReadingSettings {
	pub fn restore(
		&mut self,
		result: Result<ReadingPreferences, StoreError>,
	) -> Option<ReadingPreferences> {
		self.loaded = true;
		if self.touched {
			return None;
		}
		match result {
			Ok(value) => {
				self.current = value;
				Some(value)
			}
			Err(_) => {
				self.failed = true;
				None
			}
		}
	}
	pub fn observe(&mut self, value: ReadingPreferences, now: Instant) {
		if value.is_valid() && value != self.current {
			self.current = value;
			self.touched = true;
			self.failed = false;
			self.due = Some(now + Duration::from_millis(300));
		}
	}
	pub fn ready(&self, now: Instant) -> bool {
		!self.saving && self.due.is_some_and(|due| due <= now)
	}
	pub fn request_save(&mut self, now: Instant) {
		self.touched = true;
		self.failed = false;
		self.due = Some(now);
	}
	pub fn remaining(&self, now: Instant) -> Option<Duration> {
		(!self.saving)
			.then_some(self.due)
			.flatten()
			.map(|due| due.saturating_duration_since(now))
	}
	pub fn queued(&mut self, accepted: bool) {
		self.due = None;
		self.saving = accepted;
		self.failed = !accepted;
	}
	pub fn saved(&mut self, result: Result<(), StoreError>) {
		self.saving = false;
		self.failed = result.is_err();
	}
	pub fn needs_attention(&self) -> bool {
		self.due.is_some() || self.saving || self.failed
	}
	pub fn status(&self) -> &'static str {
		if self.failed {
			if self.touched {
				"Reading and layout could not be saved; changes exist only in this session"
			} else {
				"Saved reading and layout could not be loaded; using defaults"
			}
		} else if self.due.is_some() || self.saving {
			"Saving reading and layout…"
		} else if self.loaded || self.touched {
			"Reading and layout are saved on this device, including after logout. Reset restores defaults."
		} else {
			"Loading saved reading and layout…"
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn delayed_load_drag_coalescing_and_failure_do_not_lose_the_latest_choice() {
		let now = Instant::now();
		let mut settings = ReadingSettings::default();
		let mut value = ReadingPreferences {
			zoom_percent: 120,
			..Default::default()
		};
		settings.observe(value, now);
		assert!(
			settings
				.restore(Ok(ReadingPreferences::default()))
				.is_none()
		);
		assert_eq!(settings.current, value);
		assert!(!settings.ready(now));
		assert!(settings.ready(now + Duration::from_millis(300)));
		settings.queued(true);
		for width in 237..=360 {
			value.sidebar_width = width;
			settings.observe(value, now);
		}
		assert!(!settings.ready(now + Duration::from_secs(1)));
		settings.saved(Ok(()));
		assert!(settings.ready(now + Duration::from_secs(1)));
		assert_eq!(settings.current.sidebar_width, 360);
		settings.queued(false);
		assert!(settings.needs_attention());
		assert!(settings.remaining(now).is_none()); // No retry loop for a full/dead worker.
		settings.observe(ReadingPreferences::default(), now);
		settings.request_save(now);
		settings.queued(true);
		settings.saved(Err(StoreError::Unavailable));
		assert!(settings.needs_attention());
		assert!(settings.status().contains("could not"));
		assert_eq!(settings.current, ReadingPreferences::default());
	}
	#[test]
	fn untouched_restore_and_successful_write_settle_without_repaints() {
		let mut settings = ReadingSettings::default();
		let value = ReadingPreferences {
			show_members: false,
			..Default::default()
		};
		assert_eq!(settings.restore(Ok(value)), Some(value));
		assert!(!settings.needs_attention());
		let now = Instant::now();
		settings.observe(ReadingPreferences::default(), now);
		settings.queued(true);
		settings.saved(Ok(()));
		assert!(!settings.needs_attention());
		assert!(settings.remaining(now).is_none());
		assert!(settings.restore(Ok(value)).is_none());
	}
	#[test]
	fn explicit_default_reset_defeats_late_load_and_can_retry_after_failure() {
		let now = Instant::now();
		let mut settings = ReadingSettings::default();
		settings.request_save(now);
		assert!(
			settings
				.restore(Ok(ReadingPreferences {
					zoom_percent: 150,
					sidebar_width: 360,
					show_members: false,
					animate_gifs: false,
					smooth_scrolling: true,
					scroll_speed_percent: 100,
					hide_media_links: true,
					confirm_external_links: true,
				}))
				.is_none()
		);
		assert_eq!(settings.current, ReadingPreferences::default());
		assert!(settings.ready(now));
		settings.queued(true);
		settings.saved(Err(StoreError::Unavailable));
		settings.request_save(now);
		assert!(settings.ready(now));
		settings.queued(true);
		settings.saved(Ok(()));
		assert!(!settings.needs_attention());
	}
}
