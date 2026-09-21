//! Device-local notification choices. Account notification preferences live separately.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Device {
	pub new_message: bool,
	pub current_channel: bool,
	pub incoming_ring: bool,
	pub disable_sounds: bool,
	pub unread_badge: bool,
	pub discord_sounds: bool,
	pub mute: bool,
	pub unmute: bool,
	pub deafen: bool,
	pub undeafen: bool,
}
impl Default for Device {
	fn default() -> Self {
		Self {
			new_message: true,
			current_channel: false,
			incoming_ring: true,
			disable_sounds: false,
			unread_badge: true,
			discord_sounds: false,
			mute: true,
			unmute: true,
			deafen: true,
			undeafen: true,
		}
	}
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sound {
	Message,
	CurrentChannel,
	IncomingRing,
	Mute,
	Unmute,
	Deafen,
	Undeafen,
}
impl Device {
	pub fn allows(self, sound: Sound) -> bool {
		!self.disable_sounds
			&& match sound {
				Sound::Message => self.new_message,
				Sound::CurrentChannel => self.current_channel,
				Sound::IncomingRing => self.incoming_ring,
				Sound::Mute => self.discord_sounds && self.mute,
				Sound::Unmute => self.discord_sounds && self.unmute,
				Sound::Deafen => self.discord_sounds && self.deafen,
				Sound::Undeafen => self.discord_sounds && self.undeafen,
			}
	}
}
#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn independent_sound_switches_and_master_mute() {
		let mut settings = Device::default();
		assert!(settings.allows(Sound::Message));
		assert!(!settings.allows(Sound::CurrentChannel));
		assert!(!settings.allows(Sound::Mute));
		assert!(!settings.allows(Sound::Unmute));
		assert!(!settings.allows(Sound::Deafen));
		assert!(!settings.allows(Sound::Undeafen));
		settings.discord_sounds = true;
		assert!(settings.allows(Sound::Mute));
		assert!(settings.allows(Sound::Unmute));
		assert!(settings.allows(Sound::Deafen));
		assert!(settings.allows(Sound::Undeafen));
		settings.mute = false;
		assert!(!settings.allows(Sound::Mute));
		assert!(settings.allows(Sound::Unmute));
		settings.deafen = false;
		assert!(!settings.allows(Sound::Deafen));
		assert!(settings.allows(Sound::Undeafen));
		settings.current_channel = true;
		settings.new_message = false;
		assert!(settings.allows(Sound::CurrentChannel));
		settings.disable_sounds = true;
		for sound in [
			Sound::Message,
			Sound::CurrentChannel,
			Sound::IncomingRing,
			Sound::Mute,
			Sound::Unmute,
			Sound::Deafen,
			Sound::Undeafen,
		] {
			assert!(!settings.allows(sound));
		}
		settings.disable_sounds = false;
		assert!(settings.allows(Sound::CurrentChannel));
		assert!(!settings.allows(Sound::Message));
	}
}
