//! Device-local notification choices. Account notification preferences live separately.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Device {
	pub new_message: bool,
	pub current_channel: bool,
	pub incoming_ring: bool,
	pub outgoing_ring: bool,
	pub disable_sounds: bool,
	pub unread_badge: bool,
	pub mute: bool,
	pub unmute: bool,
	pub deafen: bool,
	pub undeafen: bool,
	pub camera_on: bool,
	pub screen_share_on: bool,
	pub user_join: bool,
	pub user_leave: bool,
	pub volume: u8,
}
impl Default for Device {
	fn default() -> Self {
		Self {
			new_message: true,
			current_channel: false,
			incoming_ring: true,
			outgoing_ring: true,
			disable_sounds: false,
			unread_badge: true,
			mute: true,
			unmute: true,
			deafen: true,
			undeafen: true,
			camera_on: true,
			screen_share_on: true,
			user_join: true,
			user_leave: true,
			volume: 75,
		}
	}
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sound {
	Message,
	CurrentChannel,
	IncomingRing,
	OutgoingRing,
	Mute,
	Unmute,
	Deafen,
	Undeafen,
	CameraOn,
	ScreenShareOn,
	UserJoin,
	UserLeave,
}
impl Device {
	pub fn allows(self, sound: Sound) -> bool {
		!self.disable_sounds
			&& self.volume > 0
			&& match sound {
				Sound::Message => self.new_message,
				Sound::CurrentChannel => self.current_channel,
				Sound::IncomingRing => self.incoming_ring,
				Sound::OutgoingRing => self.outgoing_ring,
				Sound::Mute => self.mute,
				Sound::Unmute => self.unmute,
				Sound::Deafen => self.deafen,
				Sound::Undeafen => self.undeafen,
				Sound::CameraOn => self.camera_on,
				Sound::ScreenShareOn => self.screen_share_on,
				Sound::UserJoin => self.user_join,
				Sound::UserLeave => self.user_leave,
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
		assert!(settings.allows(Sound::OutgoingRing));
		settings.outgoing_ring = false;
		assert!(!settings.allows(Sound::OutgoingRing));
		assert!(settings.allows(Sound::IncomingRing));
		settings.outgoing_ring = true;
		assert!(settings.allows(Sound::Mute));
		assert!(settings.allows(Sound::Unmute));
		assert!(settings.allows(Sound::Deafen));
		assert!(settings.allows(Sound::Undeafen));
		assert!(settings.allows(Sound::CameraOn));
		assert!(settings.allows(Sound::ScreenShareOn));
		assert!(settings.allows(Sound::UserJoin));
		assert!(settings.allows(Sound::UserLeave));
		settings.user_join = false;
		assert!(!settings.allows(Sound::UserJoin));
		assert!(settings.allows(Sound::UserLeave));
		settings.user_join = true;
		settings.user_leave = false;
		assert!(settings.allows(Sound::UserJoin));
		assert!(!settings.allows(Sound::UserLeave));
		settings.user_leave = true;
		settings.camera_on = false;
		assert!(!settings.allows(Sound::CameraOn));
		assert!(settings.allows(Sound::ScreenShareOn));
		settings.camera_on = true;
		settings.screen_share_on = false;
		assert!(settings.allows(Sound::CameraOn));
		assert!(!settings.allows(Sound::ScreenShareOn));
		settings.screen_share_on = true;
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
			Sound::OutgoingRing,
			Sound::Mute,
			Sound::Unmute,
			Sound::Deafen,
			Sound::Undeafen,
			Sound::CameraOn,
			Sound::ScreenShareOn,
			Sound::UserJoin,
			Sound::UserLeave,
		] {
			assert!(!settings.allows(sound));
		}
		settings.disable_sounds = false;
		settings.volume = 0;
		assert!(!settings.allows(Sound::CurrentChannel));
		assert!(!settings.allows(Sound::IncomingRing));
		settings.volume = 75;
		assert!(settings.allows(Sound::CurrentChannel));
		assert!(!settings.allows(Sound::Message));
	}
}
