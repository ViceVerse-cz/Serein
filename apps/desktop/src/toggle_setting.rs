/// Coalesce toggles behind one SQLite write and protect a choice from a late load.
#[derive(Default)]
pub struct Settings {
	pub enabled: bool,
	/// What `enabled` should be before a load completes, and what a load failure falls back
	/// to; an opt-out setting passes `true` here so a missing/failed override still turns on.
	default: bool,
	pub touched: bool,
	pub dirty: bool,
	pub saving: bool,
	pub failed: bool,
}
impl Settings {
	/// `default` is the value used before the stored override loads, and if loading fails.
	pub fn with_default(default: bool) -> Self {
		Self {
			enabled: default,
			default,
			..Self::default()
		}
	}
	pub fn observe(&mut self, enabled: bool) {
		if self.enabled != enabled {
			self.enabled = enabled;
			self.touched = true;
			self.dirty = true;
			self.failed = false;
		}
	}
	pub fn restore(&mut self, result: Result<bool, local_store::StoreError>) {
		if !self.touched {
			self.enabled = result.unwrap_or(self.default);
			self.failed = result.is_err();
		}
	}
	pub fn status(&self) -> &'static str {
		if self.failed {
			"Setting could not be saved or loaded. Toggle it to retry saving."
		} else if self.dirty || self.saving {
			"Saving setting…"
		} else {
			""
		}
	}
	pub fn needs_attention(&self) -> bool {
		self.dirty || self.saving || self.failed
	}
}
