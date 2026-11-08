//! Applies notification device choices to filtered messages and explicit call ringing state.
use client_core::{State, auth::AuthState};
use model::{
	Id, PresenceStatus,
	notification_preferences::{Device, Sound},
};
use std::time::{Duration, Instant};

pub enum Alert {
	Message {
		channel: Id,
		title: String,
		body: String,
		avatar_key: String,
		image_path: Option<String>,
	},
}
#[derive(Default)]
pub struct Runtime {
	sounds: crate::notification_sounds::Sounds,
	/// Push-to-talk blips use their own worker, so a press or release neither cuts off a
	/// message or ring cue nor is cancelled by one.
	ptt_sounds: crate::notification_sounds::Sounds,
	options: Device,
	was_audible: bool,
	ring: Option<((Id, Sound), Instant)>,
	ptt: PttCues,
	badge: Option<u32>,
	badge_check: Option<Instant>,
	badge_status: &'static str,
	/// Direct conversations that alerted since they were last read, with the message that
	/// alerted, for one ping per DM.
	pinged_directs: std::collections::BTreeMap<Id, Id>,
}

/// With one ping per direct conversation, keeps a conversation quiet after its first alert
/// until its read marker reaches that message; returns whether this notification stays
/// silent. `read` gives a conversation's last read message, if known. Checking records
/// nothing; [`record_ping`] does once an alert or sound was actually produced.
fn quiet_repeat(
	pinged: &mut std::collections::BTreeMap<Id, Id>,
	(direct, channel): (bool, Id),
	read: impl Fn(Id) -> Option<Id>,
) -> bool {
	pinged.retain(|channel, first| read(*channel).is_none_or(|marker| marker < *first));
	direct && pinged.contains_key(&channel)
}

/// Marks a direct conversation as having alerted with `message`, keeping its first alert.
fn record_ping(
	pinged: &mut std::collections::BTreeMap<Id, Id>,
	(direct, channel, message): (bool, Id, Id),
) {
	if direct {
		pinged.entry(channel).or_insert(message);
	}
}
/// Push-to-talk edges: one start cue per press and one stop cue per release.
#[derive(Default)]
struct PttCues {
	active: bool,
}
impl PttCues {
	/// `mode` is the push-to-talk setting, `in_call` a live voice session and `held` the key.
	/// A state without a live push-to-talk session resets silently: ending the call or turning
	/// the mode off while holding is not a key release and must not cue.
	fn update(&mut self, mode: bool, in_call: bool, held: bool) -> Option<Sound> {
		if !(mode && in_call) {
			self.active = false;
			return None;
		}
		let active = std::mem::replace(&mut self.active, held);
		(active != held).then_some(if held {
			Sound::PttStart
		} else {
			Sound::PttStop
		})
	}
}
impl Runtime {
	fn ring_cue(&mut self, ringing: Option<(Id, Sound)>, now: Instant) -> Option<Sound> {
		if self.ring.map(|(key, _)| key) != ringing {
			if self.ring.is_some() {
				self.sounds.stop();
			}
			self.ring = ringing.map(|key| (key, now));
			return ringing.map(|(_, cue)| cue);
		}
		if let Some(((_, cue), played)) = &mut self.ring {
			let interval = if *cue == Sound::OutgoingRing {
				crate::notification_sounds::OUTGOING_RING_INTERVAL
			} else {
				crate::notification_sounds::RING_INTERVAL
			};
			if now.duration_since(*played) >= interval {
				*played = now;
				return Some(*cue);
			}
		}
		None
	}
	/// Plays one local push-to-talk cue per key edge. Direct playback keeps the cue in the
	/// frame that saw the key change, leaves the settings-preview slot to previews and uses
	/// the PTT worker so an ordinary cue cannot cancel it.
	pub fn ptt(
		&mut self,
		mode: bool,
		in_call: bool,
		held: bool,
		options: Device,
		ctx: &eframe::egui::Context,
	) {
		if let Some(cue) = self.ptt.update(mode, in_call, held)
			&& options.allows(cue)
		{
			self.ptt_sounds.play(cue, options.volume, ctx);
		}
	}
	pub fn clear(&mut self, window: &winit::window::Window) {
		self.sounds.stop();
		self.ptt_sounds.stop();
		self.ring = None;
		if self.badge.is_some_and(|count| count > 0) {
			let _ = platform::badge::set(window, 0);
		}
		self.badge = None;
		// Another account may sign in next; its conversations start without alert history.
		self.pinged_directs.clear();
	}
	/// Returns a coalesced desktop alert request. Sound is independent of desktop alerts.
	pub fn poll(
		&mut self,
		state: &mut State,
		ui: &mut ui::MessagingUi,
		window: &winit::window::Window,
		ctx: &eframe::egui::Context,
		fixture: bool,
	) -> Option<Alert> {
		let live = !fixture && !state.demo && state.auth == AuthState::Authenticated;
		let audible = live && ui.own_presence.status != PresenceStatus::DoNotDisturb;
		let options = ui.notification_options;
		let badges = platform::badge::supported() && options.unread_badge;
		if options != self.options || (self.was_audible && !audible) {
			self.sounds.stop();
			self.ptt_sounds.stop();
			self.options = options;
		}
		self.was_audible = audible;
		let focused = ctx.input(|i| {
			i.focused
				&& i.viewport().visible() != Some(false)
				&& i.viewport().minimized != Some(true)
		});
		let mut alert = None;
		let mut sound = None;
		let one_ping = ui.interface_tweak_on(extensions::InterfaceTweak::OnePingPerDirectMessage);
		if !one_ping {
			self.pinged_directs.clear();
		}
		// Alerts and sounds in one poll coalesce to the last eligible one of each; only the
		// conversations behind those use up their one ping.
		let (mut alerted, mut sounded) = (None, None);
		while let Some(notification) = state.take_notification() {
			let current = focused && ui.viewing_latest(notification.channel);
			let cue = if current {
				Sound::CurrentChannel
			} else {
				Sound::Message
			};
			let desktop = audible && ui.notifications_enabled && !current;
			let audio = audible && options.allows(cue);
			let key = (
				notification.is_direct(),
				notification.channel,
				notification.message,
			);
			if !(desktop || audio)
				|| one_ping
					&& quiet_repeat(&mut self.pinged_directs, (key.0, key.1), |id| {
						state.read_marker(id).flatten()
					}) {
				continue;
			}
			if desktop {
				let image_path = state.user.as_ref().and_then(|user| {
					crate::avatars::notification_image_path(user.id, &notification.avatar_key)
				});
				alert = Some(Alert::Message {
					channel: notification.channel,
					title: notification.sender,
					body: notification.preview,
					avatar_key: notification.avatar_key,
					image_path,
				});
				alerted = Some(key);
			}
			if audio {
				sound = Some(cue);
				sounded = Some((key, cue));
			}
		}
		let incoming = state.voice.incoming.filter(|id| {
			audible && state.notification_allowed(*id) && options.allows(Sound::IncomingRing)
		});
		// Dialing is explicit local feedback, like mute/camera cues; DND only silences alerts.
		let outgoing = state
			.outgoing_ring()
			.filter(|_| live && options.allows(Sound::OutgoingRing));
		let ringing = incoming
			.map(|id| (id, Sound::IncomingRing))
			.or_else(|| outgoing.map(|id| (id, Sound::OutgoingRing)));
		if let Some(cue) = self.ring_cue(ringing, Instant::now()) {
			sound = Some(cue);
		}
		if self.ring.is_some() {
			ctx.request_repaint_after(Duration::from_millis(250));
		}
		// Explicit previews are allowed in the offline demo and intentionally ignore automatic mute choices.
		if let Some(preview) = ui.notification_preview.take() {
			sound = Some(preview);
		}
		// Only the alert shown and a message sound that survived ringtone and preview
		// selection use up a conversation's one ping.
		if one_ping {
			let heard = sounded
				.filter(|(_, cue)| sound == Some(*cue))
				.map(|(key, _)| key);
			for key in alerted.into_iter().chain(heard) {
				record_ping(&mut self.pinged_directs, key);
			}
		}
		if let Some(sound) = sound {
			self.sounds.play(sound, options.volume, ctx);
		}
		// Counts otherwise only change while frames run: a throttled frame schedules one
		// follow-up recount, and then the window can sleep until the next mute expiry.
		let since = self.badge_check.map(|time| time.elapsed());
		if since.is_none_or(|since| since >= Duration::from_secs(1)) || !badges || !live {
			self.badge_check = Some(Instant::now());
			let pings = if live && badges {
				badge_count(state)
			} else {
				0
			};
			if platform::badge::supported() && self.badge != Some(pings) {
				self.badge_status = platform::badge::set(window, pings).err().unwrap_or("");
				self.badge = Some(pings);
			}
			// A lapsing timed mute changes the count without any other activity.
			if live
				&& badges && let Some(expiry) = state.next_mute_expiry()
			{
				ctx.request_repaint_after(expiry);
			}
		} else if let Some(since) = since {
			ctx.request_repaint_after(Duration::from_secs(1).saturating_sub(since));
		}
		let ptt_error = self.ptt_sounds.error();
		let sound_status = if !ptt_error.is_empty() {
			ptt_error
		} else if self.sounds.status().is_empty() {
			self.ptt_sounds.status()
		} else {
			self.sounds.status()
		};
		ui.notification_sound_status = if sound_status.is_empty() {
			self.badge_status
		} else {
			sound_status
		};
		alert
	}
}

fn badge_count(state: &State) -> u32 {
	state
		.channels
		.iter()
		.try_fold(0u32, |total, channel| {
			if total >= 100 {
				return None;
			}
			let count = state.mention_count(channel.id);
			if count == 0 {
				return Some(total);
			}
			let muted = if channel.guild.is_some() {
				state.muted_for_guild_rail(channel)
			} else {
				state.dm_muted(channel.id) == Some(true)
			};
			Some(if muted {
				total
			} else {
				total.saturating_add(count).min(100)
			})
		})
		.unwrap_or(100)
}

#[cfg(any(test, all(debug_assertions, feature = "demo")))]
pub fn debug_badge_check() {
	use client_core::{Envelope, Event, notifications};
	let mut state = test_support::demo_state();
	state.read_state.reset();
	state.apply(Envelope {
		generation: state.generation,
		event: Event::ReadState(client_core::read_state::Event::Snapshot {
			entries: Some(vec![(Id(20), None, 3), (Id(22), None, 4)]),
			version: Some(1),
			partial: false,
		}),
	});
	assert_eq!(badge_count(&state), 7);
	for (guild, channels, muted, expected) in [
		(None, vec![(Id(22), Some(true), None)], false, 3),
		(Some(Id(10)), vec![(Id(20), Some(true), None)], false, 0),
		(None, vec![(Id(22), Some(false), None)], false, 4),
		(Some(Id(10)), vec![(Id(23), Some(true), None)], false, 4),
		(Some(Id(10)), vec![], true, 4),
		(Some(Id(10)), vec![], false, 7),
	] {
		state
			.apply_notification_preferences(notifications::Event::Settings {
				entries: vec![notifications::Setting {
					guild,
					muted: Some(muted),
					channels,
					overrides_known: true,
					..Default::default()
				}],
				replace: false,
			})
			.unwrap();
		assert_eq!(badge_count(&state), expected);
	}
	assert_eq!(state.mention_count(Id(20)), 3);
	assert_eq!(state.mention_count(Id(22)), 4);
	state
		.channels
		.iter_mut()
		.find(|c| c.id == Id(22))
		.unwrap()
		.kind = 3;
	for (until, expected) in [(i64::MAX, 3), (1, 7)] {
		state
			.apply_notification_preferences(notifications::Event::Settings {
				entries: vec![notifications::Setting {
					guild: None,
					muted: Some(false),
					channels: vec![(Id(22), Some(true), None)],
					channel_mute_until: vec![(Id(22), until)],
					..Default::default()
				}],
				replace: false,
			})
			.unwrap();
		assert_eq!(badge_count(&state), expected);
		assert_eq!(state.next_mute_expiry().is_some(), until == i64::MAX);
	}
	state.apply(Envelope {
		generation: state.generation,
		event: Event::ReadState(client_core::read_state::Event::Snapshot {
			entries: Some(vec![(Id(20), None, u32::MAX), (Id(22), None, 4)]),
			version: Some(2),
			partial: false,
		}),
	});
	assert_eq!(badge_count(&state), 100);
	println!("Notification badge debug check passed: mutes, expiry, unmute and count cap.");
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn muted_conversations_do_not_count_toward_native_badge() {
		debug_badge_check();
	}

	/// Checks that one ping per DM silences repeats until the conversation is read through the
	/// alerting message, leaves server channels alone, tracks each conversation separately and
	/// records nothing for a notification that was checked but never alerted.
	#[test]
	fn one_ping_per_direct_message_waits_for_read() {
		let mut pinged = std::collections::BTreeMap::new();
		let mut read = std::collections::BTreeMap::<Id, Id>::new();
		let check = |pinged: &mut std::collections::BTreeMap<Id, Id>,
		             read: &std::collections::BTreeMap<Id, Id>,
		             direct,
		             channel| {
			let read = read.clone();
			quiet_repeat(pinged, (direct, Id(channel)), move |id| {
				read.get(&id).copied()
			})
		};
		// A checked notification that a later one replaced in the same poll leaves no mark.
		assert!(!check(&mut pinged, &read, true, 1));
		assert!(
			!check(&mut pinged, &read, true, 1),
			"an unalerted DM stays eligible"
		);
		// The rest of the scenario alerts every eligible notification.
		let quiet = |pinged: &mut std::collections::BTreeMap<Id, Id>,
		             read: &std::collections::BTreeMap<Id, Id>,
		             direct,
		             channel,
		             message| {
			let quiet = check(pinged, read, direct, channel);
			if !quiet {
				record_ping(pinged, (direct, Id(channel), Id(message)));
			}
			quiet
		};
		assert!(
			!quiet(&mut pinged, &read, true, 1, 100),
			"first DM message alerts"
		);
		assert!(
			quiet(&mut pinged, &read, true, 1, 101),
			"repeat stays quiet"
		);
		assert!(
			!quiet(&mut pinged, &read, true, 2, 102),
			"another DM alerts"
		);
		assert!(
			!quiet(&mut pinged, &read, false, 3, 103),
			"server channel alerts"
		);
		assert!(
			!quiet(&mut pinged, &read, false, 3, 104),
			"server repeats alert"
		);
		read.insert(Id(1), Id(99));
		assert!(
			quiet(&mut pinged, &read, true, 1, 105),
			"read before the alert keeps quiet"
		);
		read.insert(Id(1), Id(101));
		assert!(
			!quiet(&mut pinged, &read, true, 1, 106),
			"reading through it resets"
		);
		assert!(quiet(&mut pinged, &read, true, 1, 107), "then quiet again");
	}

	#[test]
	fn ringtone_timer_repeats_each_cue_and_stops_on_clear() {
		let mut runtime = Runtime::default();
		let now = Instant::now();
		let outgoing = Some((Id(2), Sound::OutgoingRing));
		assert_eq!(runtime.ring_cue(outgoing, now), Some(Sound::OutgoingRing));
		assert_eq!(
			runtime.ring_cue(outgoing, now + Duration::from_secs(2)),
			None
		);
		assert_eq!(
			runtime.ring_cue(outgoing, now + Duration::from_secs(3)),
			Some(Sound::OutgoingRing)
		);
		let incoming = Some((Id(2), Sound::IncomingRing));
		assert_eq!(
			runtime.ring_cue(incoming, now + Duration::from_secs(3)),
			Some(Sound::IncomingRing)
		);
		assert_eq!(
			runtime.ring_cue(incoming, now + Duration::from_secs(6)),
			None
		);
		assert_eq!(
			runtime.ring_cue(incoming, now + Duration::from_secs(9)),
			Some(Sound::IncomingRing)
		);
		assert_eq!(runtime.ring_cue(None, now + Duration::from_secs(9)), None);
		assert!(runtime.ring.is_none());
		assert_eq!(runtime.ring_cue(None, now + Duration::from_secs(15)), None);
	}

	#[test]
	fn push_to_talk_cues_fire_once_per_press_and_release() {
		let mut cues = PttCues::default();
		assert_eq!(cues.update(true, true, false), None);
		assert_eq!(cues.update(true, true, true), Some(Sound::PttStart));
		for _ in 0..50 {
			assert_eq!(cues.update(true, true, true), None);
		}
		assert_eq!(cues.update(true, true, false), Some(Sound::PttStop));
		assert_eq!(cues.update(true, true, false), None);
		// Ending the call while held or turning the mode off is not a key release.
		assert_eq!(cues.update(true, true, true), Some(Sound::PttStart));
		assert_eq!(cues.update(true, false, true), None);
		assert_eq!(cues.update(true, true, false), None);
		// Held while connecting stays silent; becoming live mid-hold starts transmission.
		assert_eq!(cues.update(true, false, true), None);
		assert_eq!(cues.update(true, false, true), None);
		assert_eq!(cues.update(true, true, true), Some(Sound::PttStart));
		assert_eq!(cues.update(true, true, true), None);
		assert_eq!(cues.update(true, true, false), Some(Sound::PttStop));
		assert_eq!(cues.update(true, true, true), Some(Sound::PttStart));
		assert_eq!(cues.update(false, true, false), None);
		assert_eq!(cues.update(true, true, false), None);
	}
}
