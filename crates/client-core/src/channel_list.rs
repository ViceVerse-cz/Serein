//! Discord's opt-in channel list ("Browse Channels", "Remove from Channel List").
//!
//! Unofficial normal-user settings (discord-userdoccers, user guild settings): the guild flag
//! `OPT_IN_CHANNELS_ON` hides channels the account has not opted in to, `OPT_IN_CHANNELS_OFF`
//! shows all of them, and a channel override's `OPT_IN_ENABLED` opts that channel (or a whole
//! category) in. One bounded settings write is pending at a time and is confirmed before the
//! sidebar changes. Discord takes complete flag values, so the transport rebases the planned
//! opt-in bits onto the newest settings row right before sending; other bits, such as
//! favorites changed by another session meanwhile, are kept.
use crate::{
	Command, State,
	auth::{AuthState, Failure},
};
use model::{Channel, Id};
use std::{
	collections::BTreeMap,
	sync::{Arc, Mutex},
};

pub const OPT_IN_CHANNELS_OFF: u32 = 1 << 13;
pub const OPT_IN_CHANNELS_ON: u32 = 1 << 14;
pub const OPT_IN_ENABLED: u32 = 1 << 12;
/// The guild flag bits a channel list write owns; every other bit comes from the newest row.
pub const MODE_BITS: u32 = OPT_IN_CHANNELS_ON | OPT_IN_CHANNELS_OFF;
/// Discord caps a server at 500 channels; one write never names more.
pub const MAX_CHANNELS: usize = 500;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Change {
	/// Add a channel or category to the channel list, or remove it.
	List { channel: Id, listed: bool },
	/// "Show All Channels" turns opt-in mode off; clearing it turns opt-in mode on.
	ShowAll(bool),
}

/// The complete flags this write sets: the guild flags and each changed channel override.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Write {
	pub guild: Id,
	pub flags: Option<u32>,
	pub channels: Vec<(Id, u32)>,
}
impl Write {
	pub fn valid(&self) -> bool {
		let mut ids: Vec<_> = self.channels.iter().map(|(id, _)| *id).collect();
		ids.sort_unstable();
		ids.dedup();
		self.guild.0 != 0
			&& (self.flags.is_some() || !self.channels.is_empty())
			&& self.channels.len() <= MAX_CHANNELS
			&& ids.len() == self.channels.len()
			&& ids.iter().all(|id| id.0 != 0)
	}
	pub fn bytes(&self) -> usize {
		size_of::<Self>() + self.channels.capacity() * size_of::<(Id, u32)>()
	}
	/// This write's opt-in bits on top of `base`, the newest known values.
	pub fn rebase(&self, base: &Base) -> Self {
		Self {
			guild: self.guild,
			flags: self
				.flags
				.map(|flags| (base.flags & !MODE_BITS) | (flags & MODE_BITS)),
			channels: self
				.channels
				.iter()
				.map(|(id, flags)| {
					(
						*id,
						(base.channel(*id) & !OPT_IN_ENABLED) | (flags & OPT_IN_ENABLED),
					)
				})
				.collect(),
		}
	}
}

/// The newest known guild flags and the pending write's channel overrides.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Base {
	pub flags: u32,
	pub channels: Vec<(Id, u32)>,
}
impl Base {
	fn channel(&self, id: Id) -> u32 {
		self.channels
			.iter()
			.find(|(channel, _)| *channel == id)
			.map_or(0, |(_, flags)| *flags)
	}
}
/// Shared with the transport while a write is pending. `None` once a settings row with unknown
/// flags arrived; the transport then refuses to send.
pub type Latest = Arc<Mutex<Option<Base>>>;

pub enum Event {
	/// On success, the values actually sent.
	Written {
		guild: Id,
		request: u64,
		result: Result<Box<Write>, Failure>,
	},
}

#[derive(Default)]
pub struct ChannelList {
	sequence: u64,
	/// The write, its request number, whether a settings row already carried it, and the
	/// newest values shared with the transport.
	pending: Option<(Write, u64, bool, Latest)>,
	status: Option<(Id, &'static str)>,
}
impl ChannelList {
	pub(crate) fn reset(&mut self) {
		*self = Self {
			sequence: self.sequence,
			..Self::default()
		};
	}
}

impl State {
	/// `Some(true)` when the server shows only channels this account opted in to.
	pub fn channel_list_opt_in(&self, guild: Id) -> Option<bool> {
		self.notification_preferences
			.setting(Some(guild))
			.and_then(|setting| setting.flags)
			.map(|flags| flags & OPT_IN_CHANNELS_ON != 0)
	}
	fn opted_in(&self, guild: Id, channel: Id) -> bool {
		self.notification_preferences
			.setting(Some(guild))
			.and_then(|setting| setting.channel_flags(channel))
			.is_some_and(|flags| flags & OPT_IN_ENABLED != 0)
	}
	/// Whether the sidebar lists this channel. Outside opt-in mode, or while the settings are
	/// unknown, every channel is listed. Threads follow their parent; the open conversation and
	/// the active call stay listed so removing them never strands the view.
	pub fn channel_listed(&self, channel: &Channel) -> bool {
		let Some(guild) = channel.guild else {
			return true;
		};
		if self.channel_list_opt_in(guild) != Some(true) {
			return true;
		}
		if matches!(channel.kind, 10..=12) {
			return channel
				.parent_id
				.and_then(|id| self.channel(id))
				.is_none_or(|parent| parent.id == channel.id || self.channel_listed(parent));
		}
		let focused = self.selected.and_then(|id| self.channel(id)).map(|c| {
			if matches!(c.kind, 10..=12) {
				c.parent_id.unwrap_or(c.id)
			} else {
				c.id
			}
		});
		self.opted_in(guild, channel.id)
			|| (channel.kind != 4
				&& channel
					.parent_id
					.is_some_and(|parent| self.opted_in(guild, parent)))
			|| focused == Some(channel.id)
			|| self
				.voice
				.active
				.as_ref()
				.is_some_and(|call| call.channel == channel.id)
	}
	/// Whether the account explicitly opted in to this channel or category, ignoring the
	/// focused-channel exception. Browse Channels shows this as the checkbox state.
	pub fn channel_followed(&self, channel: &Channel) -> bool {
		let Some(guild) = channel.guild else {
			return true;
		};
		self.channel_list_opt_in(guild) != Some(true)
			|| self.opted_in(guild, channel.id)
			|| (channel.kind != 4
				&& channel
					.parent_id
					.is_some_and(|parent| self.opted_in(guild, parent)))
	}
	pub fn channel_list_pending(&self) -> bool {
		self.channel_list.pending.is_some()
	}
	pub fn channel_list_status(&self, guild: Id) -> Option<&'static str> {
		self.channel_list
			.status
			.filter(|(id, _)| *id == guild)
			.map(|(_, status)| status)
	}
	pub fn can_change_channel_list(&self, guild: Id) -> bool {
		guild.0 != 0
			&& self.guild(guild).is_some()
			&& (self.demo || (self.auth == AuthState::Authenticated && self.gateway_connected))
			&& !self.channel_list_pending()
			&& self
				.notification_preferences
				.setting(Some(guild))
				.is_some_and(|setting| setting.overrides_known && setting.flags.is_some())
	}
	/// Plans the flags Discord's client would write for `change`. Leaving show-all mode first
	/// opts in to every category and uncategorized channel, so only the removed item disappears.
	pub fn plan_channel_list(&self, guild: Id, change: Change) -> Option<Write> {
		let setting = self.notification_preferences.setting(Some(guild))?;
		let current = setting.flags?;
		if !setting.overrides_known {
			return None;
		}
		let flags_of = |id: Id| setting.channel_flags(id).unwrap_or(0);
		let listable = |c: &&Channel| {
			c.guild == Some(guild) && !matches!(c.kind, 10..=12) && self.can_view(c.id)
		};
		let mut changed = BTreeMap::<Id, u32>::new();
		let effective = |changed: &BTreeMap<Id, u32>, id: Id| {
			changed.get(&id).copied().unwrap_or_else(|| flags_of(id))
		};
		let set = |changed: &mut BTreeMap<Id, u32>, id: Id, on: bool| {
			let before = effective(changed, id);
			let after = if on {
				before | OPT_IN_ENABLED
			} else {
				before & !OPT_IN_ENABLED
			};
			changed.insert(id, after);
		};
		let opt_in = current & OPT_IN_CHANNELS_ON != 0;
		// Seed only a server that never used the opt-in list (neither mode bit set and nothing
		// opted in, including channels this account cannot see now). A saved selection, even an
		// empty one, survives Show All Channels.
		let fresh = current & (OPT_IN_CHANNELS_ON | OPT_IN_CHANNELS_OFF) == 0
			&& !setting
				.channel_flags
				.iter()
				.any(|(_, flags)| flags & OPT_IN_ENABLED != 0);
		let seed = |changed: &mut BTreeMap<Id, u32>| {
			let categories: Vec<Id> = self
				.channels
				.iter()
				.filter(listable)
				.filter(|c| c.kind == 4)
				.map(|c| c.id)
				.collect();
			for channel in self.channels.iter().filter(listable) {
				let top = channel.kind == 4
					|| channel
						.parent_id
						.is_none_or(|parent| !categories.contains(&parent));
				if top {
					set(changed, channel.id, true);
				}
			}
		};
		let flags = match change {
			Change::ShowAll(true) => (current & !OPT_IN_CHANNELS_ON) | OPT_IN_CHANNELS_OFF,
			Change::ShowAll(false) => {
				if fresh {
					seed(&mut changed);
				}
				(current & !OPT_IN_CHANNELS_OFF) | OPT_IN_CHANNELS_ON
			}
			Change::List { channel, listed } => {
				let target = self.channel(channel).filter(|c| listable(c))?;
				if !opt_in && fresh {
					seed(&mut changed);
				}
				if target.kind == 4 {
					set(&mut changed, target.id, listed);
					if !listed {
						for child in self
							.channels
							.iter()
							.filter(listable)
							.filter(|c| c.parent_id == Some(target.id))
						{
							set(&mut changed, child.id, false);
						}
					}
				} else if listed {
					set(&mut changed, target.id, true);
				} else {
					set(&mut changed, target.id, false);
					// A followed category lists all its channels; follow the siblings instead.
					if let Some(parent) = target
						.parent_id
						.filter(|id| effective(&changed, *id) & OPT_IN_ENABLED != 0)
					{
						set(&mut changed, parent, false);
						for sibling in self.channels.iter().filter(listable).filter(|c| {
							c.parent_id == Some(parent) && c.id != target.id && c.kind != 4
						}) {
							set(&mut changed, sibling.id, true);
						}
					}
				}
				(current & !OPT_IN_CHANNELS_OFF) | OPT_IN_CHANNELS_ON
			}
		};
		let channels: Vec<_> = changed
			.into_iter()
			.filter(|(id, value)| *value != flags_of(*id))
			.collect();
		let write = Write {
			guild,
			flags: (flags != current).then_some(flags),
			channels,
		};
		write.valid().then_some(write)
	}
	pub fn change_channel_list(&mut self, guild: Id, change: Change) -> Option<Command> {
		if !self.can_change_channel_list(guild) {
			self.channel_list.status = Some((guild, "Channel list settings are not loaded yet"));
			return None;
		}
		let Some(write) = self.plan_channel_list(guild, change) else {
			self.channel_list.status = Some((guild, "Channel list unchanged"));
			return None;
		};
		let setting = self.notification_preferences.setting(Some(guild))?;
		let base = Base {
			flags: setting.flags?,
			channels: write
				.channels
				.iter()
				.map(|(id, _)| (*id, setting.channel_flags(*id).unwrap_or(0)))
				.collect(),
		};
		let latest = Arc::new(Mutex::new(Some(base)));
		self.channel_list.sequence = self.channel_list.sequence.wrapping_add(1);
		let request = self.channel_list.sequence;
		self.channel_list.status = None;
		self.channel_list.pending = Some((write.clone(), request, false, latest.clone()));
		Some(Command::ChannelList {
			request,
			write: Box::new(write),
			latest,
		})
	}
	/// Each settings row for the pending guild becomes the base the transport sends from. A
	/// row whose opt-in bits already match the write supersedes the local confirmation; an
	/// older row leaves it to run on success.
	pub(crate) fn observe_channel_list_settings(
		&mut self,
		entries: &[crate::notifications::Setting],
	) {
		let Some((write, _, observed, latest)) = &mut self.channel_list.pending else {
			return;
		};
		for s in entries.iter().filter(|s| s.guild == Some(write.guild)) {
			// Unknown flags (an override with null flags) never stand in for the write.
			let base = s.flags.map(|flags| Base {
				flags,
				channels: write
					.channels
					.iter()
					.map(|(id, _)| {
						let row = s.channel_flags.iter().find(|(channel, _)| channel == id);
						(*id, row.map_or(0, |(_, value)| *value))
					})
					.collect(),
			});
			*observed |= base.as_ref().is_some_and(|base| {
				write
					.flags
					.is_none_or(|flags| base.flags & MODE_BITS == flags & MODE_BITS)
					&& write.channels.iter().all(|(id, flags)| {
						base.channel(*id) & OPT_IN_ENABLED == flags & OPT_IN_ENABLED
					})
			});
			if let Ok(mut latest) = latest.lock() {
				*latest = base;
			}
		}
	}
	pub(crate) fn cancel_channel_list(&mut self) {
		if let Some((write, ..)) = self.channel_list.pending.take() {
			self.channel_list.status = Some((
				write.guild,
				"Outcome unknown; check Discord before retrying",
			));
		}
	}
	pub(crate) fn apply_channel_list(&mut self, event: Event) -> Result<(), &'static str> {
		let Event::Written {
			guild,
			request,
			result,
		} = event;
		let Some((write, sequence, ..)) = &self.channel_list.pending else {
			return Ok(());
		};
		if write.guild != guild || *sequence != request {
			return Ok(());
		}
		let Some((_, _, observed, _)) = self.channel_list.pending.take() else {
			return Ok(());
		};
		let status = match result {
			Err(failure) => {
				if failure.ends_session() {
					self.fail(failure);
				}
				failure.label()
			}
			Ok(sent) => {
				if !observed {
					self.confirm_channel_list(&sent)?;
				}
				"Channel list updated"
			}
		};
		self.channel_list.status = Some((guild, status));
		self.status = status;
		Ok(())
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::{Envelope, Event as CoreEvent, notifications};

	fn channel(id: u64, kind: u8, parent: Option<u64>) -> Channel {
		Channel {
			user_limit: None,
			id: Id(id),
			guild: Some(Id(100)),
			name: format!("c{id}"),
			kind,
			parent_id: parent.map(Id),
			position: id as i32,
			recipients: vec![],
			last_message: None,
			icon: None,
			member_list_id: None,
			tags: None,
			message_count: None,
		}
	}
	fn state(flags: u32, channel_flags: Vec<(Id, u32)>) -> State {
		let mut state = State {
			demo: true,
			gateway_connected: true,
			user: Some(model::User {
				primary_guild: None,
				cosmetics: None,
				id: Id(1),
				name: "Synthetic".into(),
				avatar: None,
				webhook: false,
				kind: Default::default(),
				discriminator: 0,
			}),
			guilds: vec![model::Guild {
				default_message_notifications: None,
				stickers: None,
				emojis: None,
				id: Id(100),
				name: "Synthetic".into(),
				icon: None,
			}],
			// 1 and 4 are categories; 5 is an uncategorized channel.
			channels: vec![
				channel(1, 4, None),
				channel(2, 0, Some(1)),
				channel(3, 0, Some(1)),
				channel(4, 4, None),
				channel(6, 0, Some(4)),
				channel(5, 0, None),
			],
			..State::default()
		};
		state.invalidate_navigation();
		state
			.permissions
			.replace(model::permissions::Snapshot {
				guilds: vec![model::permissions::Guild {
					id: Id(100),
					owner: Some(Id(1)),
					roles: None,
					member: None,
				}],
				channels: state
					.channels
					.iter()
					.map(|c| model::permissions::Channel {
						id: c.id,
						guild: Id(100),
						overwrites: Some(vec![]),
					})
					.collect(),
			})
			.unwrap();
		state
			.apply_notification_preferences(notifications::Event::Settings {
				entries: vec![notifications::Setting {
					overrides_known: true,
					guild: Some(Id(100)),
					channels: channel_flags
						.iter()
						.map(|(id, _)| (*id, None, None))
						.collect(),
					flags: Some(flags),
					channel_flags,
					..Default::default()
				}],
				replace: true,
			})
			.unwrap();
		state
	}
	fn listed(state: &State) -> Vec<u64> {
		state
			.channels
			.iter()
			.filter(|c| c.kind != 4 && state.channel_listed(c))
			.map(|c| c.id.0)
			.collect()
	}
	fn write(state: &mut State, change: Change) -> Write {
		let Some(Command::ChannelList { request, write, .. }) =
			state.change_channel_list(Id(100), change)
		else {
			panic!("no write planned");
		};
		let planned = (*write).clone();
		state.apply(Envelope {
			generation: state.generation,
			event: CoreEvent::ChannelList(Event::Written {
				guild: Id(100),
				request,
				result: Ok(write),
			}),
		});
		planned
	}

	#[test]
	fn first_removal_keeps_every_other_channel_listed() {
		let mut state = state(0, vec![]);
		assert_eq!(listed(&state), [2, 3, 6, 5]);
		let sent = write(
			&mut state,
			Change::List {
				channel: Id(3),
				listed: false,
			},
		);
		assert_eq!(sent.flags, Some(OPT_IN_CHANNELS_ON));
		// Category 1 is split into its remaining channel; category 4 stays followed.
		assert_eq!(
			sent.channels,
			[
				(Id(2), OPT_IN_ENABLED),
				(Id(4), OPT_IN_ENABLED),
				(Id(5), OPT_IN_ENABLED)
			]
		);
		assert_eq!(state.channel_list_opt_in(Id(100)), Some(true));
		assert_eq!(listed(&state), [2, 6, 5]);
		write(
			&mut state,
			Change::List {
				channel: Id(3),
				listed: true,
			},
		);
		assert_eq!(listed(&state), [2, 3, 6, 5]);
	}

	#[test]
	fn other_override_bits_survive_and_show_all_keeps_choices() {
		// FAVORITED (1 << 11) and the guild's unread flag (1 << 11) are preserved.
		let mut state = state(
			OPT_IN_CHANNELS_ON | 1 << 11,
			vec![(Id(5), OPT_IN_ENABLED | 1 << 11), (Id(2), 1 << 11)],
		);
		assert_eq!(listed(&state), [5]);
		let sent = write(
			&mut state,
			Change::List {
				channel: Id(2),
				listed: true,
			},
		);
		assert_eq!(sent.flags, None);
		assert_eq!(sent.channels, [(Id(2), OPT_IN_ENABLED | 1 << 11)]);
		let sent = write(&mut state, Change::ShowAll(true));
		assert_eq!(sent.flags, Some(OPT_IN_CHANNELS_OFF | 1 << 11));
		assert!(sent.channels.is_empty());
		assert_eq!(listed(&state), [2, 3, 6, 5]);
		write(&mut state, Change::ShowAll(false));
		assert_eq!(listed(&state), [2, 5]);
		// Removing a channel while showing all keeps the saved selection instead of reseeding.
		write(&mut state, Change::ShowAll(true));
		write(
			&mut state,
			Change::List {
				channel: Id(2),
				listed: false,
			},
		);
		assert_eq!(listed(&state), [5]);
		// An empty selection saved before Show All Channels stays empty.
		let mut empty = self::state(OPT_IN_CHANNELS_OFF, vec![]);
		write(&mut empty, Change::ShowAll(false));
		assert!(listed(&empty).is_empty());
		// So does one that only follows a channel this account can't see (99).
		let mut hidden = self::state(0, vec![(Id(99), OPT_IN_ENABLED)]);
		write(&mut hidden, Change::ShowAll(false));
		assert!(listed(&hidden).is_empty());
	}

	#[test]
	fn a_row_with_unknown_flags_stays_read_only_after_a_pending_write() {
		let mut state = state(
			OPT_IN_CHANNELS_ON,
			vec![(Id(2), OPT_IN_ENABLED), (Id(5), OPT_IN_ENABLED)],
		);
		let Some(Command::ChannelList {
			request,
			write,
			latest,
		}) = state.change_channel_list(
			Id(100),
			Change::List {
				channel: Id(2),
				listed: false,
			},
		)
		else {
			panic!()
		};
		assert_eq!(
			(write.flags, &write.channels[..]),
			(None, &[(Id(2), 0)][..])
		);
		// The gateway maps a row with a null override flag to unknown guild flags and leaves
		// that override out; it must neither echo the write nor become writable.
		state
			.apply_notification_preferences(notifications::Event::Settings {
				entries: vec![notifications::Setting {
					overrides_known: true,
					guild: Some(Id(100)),
					channels: vec![(Id(2), None, None), (Id(5), None, None)],
					flags: None,
					channel_flags: vec![(Id(5), OPT_IN_ENABLED)],
					..Default::default()
				}],
				replace: false,
			})
			.unwrap();
		// The transport refuses to send from unknown values.
		assert_eq!(*latest.lock().unwrap(), None);
		state.apply(Envelope {
			generation: state.generation,
			event: CoreEvent::ChannelList(Event::Written {
				guild: Id(100),
				request,
				result: Ok(write),
			}),
		});
		let setting = state
			.notification_preferences
			.setting(Some(Id(100)))
			.unwrap();
		assert_eq!(setting.channel_flags(Id(2)), Some(0));
		assert_eq!(state.channel_list_opt_in(Id(100)), None);
		assert!(!state.can_change_channel_list(Id(100)));
	}

	#[test]
	fn removing_a_category_unlists_its_channels_but_not_the_open_one() {
		let mut state = state(OPT_IN_CHANNELS_ON, vec![(Id(1), OPT_IN_ENABLED)]);
		assert_eq!(listed(&state), [2, 3]);
		state.selected = Some(Id(3));
		write(
			&mut state,
			Change::List {
				channel: Id(1),
				listed: false,
			},
		);
		assert_eq!(listed(&state), [3]);
		assert!(!state.channel_followed(state.channel(Id(3)).unwrap()));
	}

	#[test]
	fn unknown_settings_list_everything_and_refuse_writes() {
		let mut state = state(0, vec![]);
		state
			.apply_notification_preferences(notifications::Event::Invalidate)
			.unwrap();
		assert_eq!(listed(&state), [2, 3, 6, 5]);
		assert!(
			state
				.change_channel_list(Id(100), Change::ShowAll(false))
				.is_none()
		);
	}

	#[test]
	fn failed_writes_and_stale_rows_leave_the_planned_state() {
		let mut state = state(0, vec![]);
		let Some(Command::ChannelList { request, .. }) = state.change_channel_list(
			Id(100),
			Change::List {
				channel: Id(5),
				listed: false,
			},
		) else {
			panic!()
		};
		assert!(
			state
				.change_channel_list(Id(100), Change::ShowAll(true))
				.is_none()
		);
		state.apply(Envelope {
			generation: state.generation,
			event: CoreEvent::ChannelList(Event::Written {
				guild: Id(100),
				request,
				result: Err(Failure::Ambiguous),
			}),
		});
		assert_eq!(state.channel_list_opt_in(Id(100)), Some(false));
		let Some(Command::ChannelList {
			request,
			write,
			latest,
		}) = state.change_channel_list(Id(100), Change::ShowAll(false))
		else {
			panic!()
		};
		// A row from another session (here setting bit 11) doesn't stand in for the
		// confirmation, and the write is rebased on it instead of restoring the planned value.
		state
			.apply_notification_preferences(notifications::Event::Settings {
				entries: vec![notifications::Setting {
					overrides_known: true,
					guild: Some(Id(100)),
					flags: Some(OPT_IN_CHANNELS_OFF | 1 << 11),
					..Default::default()
				}],
				replace: false,
			})
			.unwrap();
		let sent = write.rebase(latest.lock().unwrap().as_ref().unwrap());
		assert_eq!(sent.flags, Some(OPT_IN_CHANNELS_ON | 1 << 11));
		state.apply(Envelope {
			generation: state.generation,
			event: CoreEvent::ChannelList(Event::Written {
				guild: Id(100),
				request,
				result: Ok(Box::new(sent)),
			}),
		});
		assert_eq!(state.channel_list_opt_in(Id(100)), Some(true));
		assert_eq!(
			state
				.notification_preferences
				.setting(Some(Id(100)))
				.and_then(|s| s.flags),
			Some(OPT_IN_CHANNELS_ON | 1 << 11)
		);
	}
}
