//! Bounded session activity; badge counts never use snowflake subtraction.
use crate::{MAX_NAV, State};
use model::{Id, Message};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

const MAX_OBSERVED: usize = 4096;
const MAX_OBSERVED_BYTES: usize = 128 * 1024;
const MAX_NOTIFICATIONS: usize = 32;
const MAX_NOTIFICATION_BYTES: usize = 16 * 1024;
const MAX_SETTINGS_BYTES: usize = 32 * 1024 * 1024;

pub struct Notification {
	pub channel: Id,
	pub message: Id,
	pub sender: String,
	pub preview: String,
	pub avatar_key: String,
	direct: bool,
	everyone: bool,
	roles: Vec<Id>,
}
impl Notification {
	fn bytes(&self) -> usize {
		size_of::<Self>()
			+ self.roles.capacity() * size_of::<Id>()
			+ self.sender.capacity()
			+ self.preview.capacity()
			+ self.avatar_key.capacity()
	}
}

fn alert_text(text: &str, chars: usize, bytes: usize) -> String {
	let mut output = String::new();
	for character in text.chars().take(chars) {
		let character = if character.is_control() || character.is_whitespace() {
			' '
		} else {
			character
		};
		if output.len() + character.len_utf8() > bytes {
			break;
		}
		output.push(character);
	}
	output.trim().to_owned()
}
#[derive(Default)]
pub(crate) struct Activity {
	counts: BTreeMap<Id, u32>,
	high_water: BTreeMap<Id, Id>,
	observed_counts: BTreeMap<Id, (u32, u32)>,
	observed: VecDeque<(Id, Id, bool)>,
	notifications: VecDeque<Notification>,
}
impl Activity {
	pub(crate) fn clear_notifications(&mut self) {
		self.notifications.clear();
	}
	pub(crate) fn forget(&mut self, channel: Id) {
		self.high_water.remove(&channel);
		self.revoke(channel);
	}
	fn revoke(&mut self, channel: Id) {
		self.counts.remove(&channel);
		self.observed_counts.remove(&channel);
		self.observed.retain(|(c, ..)| *c != channel);
		self.notifications.retain(|n| n.channel != channel);
	}
	pub(crate) fn delete(&mut self, channel: Id, message: Id) {
		self.observed
			.retain(|(c, id, _)| (*c, *id) != (channel, message));
		self.notifications
			.retain(|n| (n.channel, n.message) != (channel, message));
		if self.observed_counts.contains_key(&channel) {
			self.recount(channel);
		}
	}
	pub(crate) fn observe_latest(&mut self, channel: Id, message: Id) {
		let latest = self.high_water.entry(channel).or_insert(message);
		*latest = (*latest).max(message);
	}
	pub(crate) fn set_count(&mut self, channel: Id, count: u32) {
		self.counts.insert(channel, count);
	}
	pub(crate) fn ack(&mut self, channel: Id, message: Option<Id>, count: Option<u32>) {
		self.observed
			.retain(|(c, id, _)| *c != channel || message.is_none_or(|read| *id > read));
		self.notifications
			.retain(|n| n.channel != channel || message.is_none_or(|read| n.message > read));
		// The service count includes messages after this ACK. Avoid counting those twice.
		if let Some(count) = count {
			self.counts.insert(channel, count);
			self.observed.retain(|(c, ..)| *c != channel);
		} else {
			// Without a refreshed service count, only retained observed messages are countable.
			self.counts.remove(&channel);
		}
		self.recount(channel);
	}
	fn recount(&mut self, channel: Id) {
		let mut counts = (0, 0);
		for (c, _, mention) in &self.observed {
			if *c == channel {
				counts.0 += 1;
				counts.1 += u32::from(*mention);
			}
		}
		if counts.0 == 0 {
			self.observed_counts.remove(&channel);
		} else {
			self.observed_counts.insert(channel, counts);
		}
	}
	fn count(&self, channel: Id, mentions_only: bool) -> u32 {
		let counts = self
			.observed_counts
			.get(&channel)
			.copied()
			.unwrap_or_default();
		self.counts
			.get(&channel)
			.copied()
			.unwrap_or(0)
			.saturating_add(if mentions_only { counts.1 } else { counts.0 })
	}
}

#[derive(Clone, Default)]
pub struct Setting {
	pub guild: Option<Id>,
	pub muted: Option<bool>,
	pub level: Option<u8>,
	pub suppress_everyone: Option<bool>,
	pub suppress_roles: Option<bool>,
	pub hide_muted_channels: Option<bool>,
	pub channels: Vec<(Id, Option<bool>, Option<u8>)>,
	pub channel_mute_until: Vec<(Id, i64)>,
}
impl Setting {
	fn bytes(&self) -> usize {
		size_of::<Self>()
			+ self.channels.capacity() * size_of::<(Id, Option<bool>, Option<u8>)>()
			+ self.channel_mute_until.capacity() * size_of::<(Id, i64)>()
	}
}
pub enum Event {
	Settings {
		entries: Vec<Setting>,
		replace: bool,
	},
	Presence(Option<bool>), // Some(true) means at least one session is DND.
	Invalidate,
}
impl Event {
	pub fn bytes(&self) -> usize {
		match self {
			Self::Settings { entries, .. } => {
				entries.capacity() * size_of::<Setting>()
					+ entries
						.iter()
						.map(|e| {
							e.channels.capacity() * size_of::<(Id, Option<bool>, Option<u8>)>()
								+ e.channel_mute_until.capacity() * size_of::<(Id, i64)>()
						})
						.sum::<usize>()
			}
			_ => 0,
		}
	}
}
#[derive(Default)]
pub struct Preferences {
	settings: BTreeMap<Option<Id>, Setting>,
	dnd: Option<bool>,
}
impl State {
	pub(crate) fn invalidate_startup_preferences(&mut self, settings: bool, sessions: bool) {
		if settings {
			self.notification_preferences.settings.clear();
		}
		if sessions {
			self.notification_preferences.dnd = None;
		}
		self.read_state.activity.clear_notifications();
	}
	fn check_notification_capacity(&mut self) -> Result<(), &'static str> {
		let settings = &self.notification_preferences.settings;
		if settings.len() > MAX_NAV
			|| settings
				.values()
				.map(|setting| setting.bytes() + 64)
				.sum::<usize>()
				> MAX_SETTINGS_BYTES
		{
			self.notification_preferences.settings.clear();
			self.startup_warnings.notifications = true;
			self.read_state.activity.clear_notifications();
			return Err("Notification settings exceed safe byte capacity");
		}
		Ok(())
	}
	pub fn lights_guild_rail(&self, channel: &model::Channel) -> bool {
		channel.kind != 2
			&& self.channel_unread(channel) == Some(true)
			&& !self.muted_for_guild_rail(channel)
	}

	pub fn muted_for_guild_rail(&self, channel: &model::Channel) -> bool {
		let Some(guild) = channel.guild else {
			return false;
		};
		if self
			.notification_preferences
			.settings
			.get(&Some(guild))
			.is_some_and(|setting| setting.muted == Some(true))
		{
			return true;
		}
		let mut current = Some(channel.id);
		for _ in 0..8 {
			let Some(id) = current else {
				break;
			};
			if self.guild_channel_muted(id) == Some(true) {
				return true;
			}
			current = self
				.channel(id)
				.and_then(|c| c.parent_id.filter(|p| *p != id));
		}
		false
	}
	pub fn guild_channel_muted(&self, channel: Id) -> Option<bool> {
		let guild = self.channel(channel)?.guild?;
		let setting = self.notification_preferences.settings.get(&Some(guild));
		let muted = setting.and_then(|s| {
			s.channels
				.iter()
				.find(|(id, ..)| *id == channel)
				.and_then(|(_, muted, _)| *muted)
		});
		if muted == Some(true)
			&& setting.is_some_and(|s| {
				s.channel_mute_until
					.iter()
					.any(|(id, until)| *id == channel && *until <= Self::permission_time())
			}) {
			return Some(false);
		}
		muted.or_else(|| (self.demo || setting.is_some()).then_some(false))
	}
	pub fn channel_notification_level(&self, channel: Id) -> Option<u8> {
		let guild = self.channel(channel)?.guild?;
		let setting = self.notification_preferences.settings.get(&Some(guild))?;
		setting
			.channels
			.iter()
			.find(|(id, ..)| *id == channel)
			.and_then(|(_, _, level)| *level)
			.or(Some(3))
	}
	pub(crate) fn confirm_channel_mute_timer(
		&mut self,
		channel: Id,
		muted: Option<bool>,
		until: Option<i64>,
	) -> Result<(), &'static str> {
		let Some(guild) = self.channel(channel).and_then(|c| c.guild) else {
			return Ok(());
		};
		let Some(setting) = self.notification_preferences.settings.get_mut(&Some(guild)) else {
			return Ok(());
		};
		setting.channel_mute_until.retain(|(id, _)| *id != channel);
		if muted == Some(true)
			&& let Some(until) = until
		{
			if setting.channel_mute_until.len() >= MAX_NAV {
				return Err("Mute timers exceed safe capacity");
			}
			setting.channel_mute_until.push((channel, until));
		}
		self.check_notification_capacity()
	}
	pub(crate) fn confirm_channel_preferences(
		&mut self,
		guild: Id,
		channel: Id,
		muted: Option<bool>,
		level: Option<u8>,
	) -> Result<(), &'static str> {
		let preferences = &mut self.notification_preferences;
		let existing = preferences.settings.get(&Some(guild));
		if existing.is_none() && preferences.settings.len() >= MAX_NAV {
			return Err("Notification settings exceed safe capacity");
		}
		if !existing.is_some_and(|s| s.channels.iter().any(|(id, ..)| *id == channel))
			&& preferences
				.settings
				.values()
				.map(|s| s.channels.len())
				.sum::<usize>()
				>= MAX_NAV
		{
			return Err("Notification settings exceed safe capacity");
		}
		let setting = preferences
			.settings
			.entry(Some(guild))
			.or_insert_with(|| Setting {
				guild: Some(guild),
				..Setting::default()
			});
		if let Some((_, m, l)) = setting.channels.iter_mut().find(|(id, ..)| *id == channel) {
			if muted.is_some() {
				*m = muted;
			}
			if level.is_some() {
				*l = level;
			}
		} else {
			setting.channels.push((channel, muted, level));
		}
		self.read_state.activity.clear_notifications();
		self.check_notification_capacity()
	}

	pub fn hides_muted_channels(&self, guild: Id) -> Option<bool> {
		self.notification_preferences
			.settings
			.get(&Some(guild))
			.and_then(|setting| setting.hide_muted_channels)
	}

	pub(crate) fn confirm_guild_hides_muted(
		&mut self,
		guild: Id,
		hide: bool,
	) -> Result<(), &'static str> {
		let preferences = &mut self.notification_preferences;
		if !preferences.settings.contains_key(&Some(guild)) && preferences.settings.len() >= MAX_NAV
		{
			return Err("Notification settings exceed safe capacity");
		}
		let setting = preferences
			.settings
			.entry(Some(guild))
			.or_insert_with(|| Setting {
				guild: Some(guild),
				..Setting::default()
			});
		setting.hide_muted_channels = Some(hide);
		self.check_notification_capacity()
	}

	/// Per-DM notification override; absent settings remain unknown outside the fixture.
	pub fn dm_muted(&self, channel: Id) -> Option<bool> {
		if let Some(muted) = self.pending_dm_muted(channel) {
			return Some(muted);
		}
		let Some(setting) = self.notification_preferences.settings.get(&None) else {
			return self.demo.then_some(false);
		};
		setting
			.channels
			.iter()
			.find(|(id, ..)| *id == channel)
			.map(|(_, muted, _)| *muted)
			.unwrap_or_else(|| (self.demo || setting.muted.is_some()).then_some(false))
	}
	pub(crate) fn confirm_dm_muted(
		&mut self,
		channel: Id,
		muted: bool,
	) -> Result<(), &'static str> {
		let existing = self.notification_preferences.settings.get(&None);
		if !existing.is_some_and(|s| s.channels.iter().any(|(id, ..)| *id == channel))
			&& self
				.notification_preferences
				.settings
				.values()
				.map(|s| s.channels.len())
				.sum::<usize>()
				>= MAX_NAV
		{
			return Err("Notification settings exceed safe capacity");
		}
		let setting = self
			.notification_preferences
			.settings
			.entry(None)
			.or_default();
		if let Some((_, value, _)) = setting.channels.iter_mut().find(|(id, ..)| *id == channel) {
			*value = Some(muted);
		} else {
			setting.channels.push((channel, Some(muted), None));
		}
		self.read_state.activity.clear_notifications();
		self.check_notification_capacity()
	}
	/// Latest known message activity, retained across deletion for navigation ordering.
	pub fn channel_activity(&self, channel: &model::Channel) -> Id {
		self.read_state
			.activity
			.high_water
			.get(&channel.id)
			.copied()
			.max(channel.last_message)
			.unwrap_or(channel.id)
	}
	pub fn unread_directs(&self, call: Option<Id>) -> Vec<Id> {
		let mut rows: Vec<(Id, Id)> = self
			.channels
			.iter()
			.filter(|channel| {
				channel.guild.is_none()
					&& channel.supports_text()
					&& !self.message_request_pending(channel.id)
					&& !self.spam_direct(channel.id)
					&& (Some(channel.id) == call
						|| self.channel_unread(channel) == Some(true)
						|| self.unread_count(channel.id) > 0)
			})
			.map(|channel| (self.channel_activity(channel), channel.id))
			.collect();
		rows.sort_unstable_by_key(|&(activity, id)| std::cmp::Reverse((activity, id)));
		if let Some(call) = call
			&& let Some(index) = rows.iter().position(|(_, id)| *id == call)
		{
			rows[..=index].rotate_right(1);
		}
		rows.into_iter().take(15).map(|(_, id)| id).collect()
	}
	/// Service badge count plus bounded activity observed since that count. A lower bound
	/// when history or settings are incomplete; this is not an exact total unread count.
	pub fn unread_count(&self, channel: Id) -> u32 {
		if self.can_view(channel) {
			self.read_state.activity.count(channel, false)
		} else {
			0
		}
	}
	pub fn mention_count(&self, channel: Id) -> u32 {
		if self.can_view(channel) {
			self.read_state.activity.count(channel, true)
		} else {
			0
		}
	}
	pub(crate) fn reconcile_notifications(&mut self) {
		let activity = &self.read_state.activity;
		let revoked: BTreeSet<_> = activity
			.counts
			.keys()
			.chain(activity.observed_counts.keys())
			.copied()
			.chain(
				activity
					.notifications
					.iter()
					.map(|notification| notification.channel),
			)
			.filter(|channel| !self.can_view(*channel))
			.collect();
		for channel in revoked {
			self.read_state.activity.revoke(channel);
		}
	}
	pub fn take_notification(&mut self) -> Option<Notification> {
		while let Some(notification) = self.read_state.activity.notifications.pop_front() {
			let mention = self.mention_matches(
				notification.channel,
				notification.direct,
				notification.everyone,
				&notification.roles,
			);
			if self.notification_allowed_for(notification.channel, mention) {
				return Some(notification);
			}
		}
		None
	}
	pub fn notification_preferences_known(&self) -> bool {
		self.notification_preferences.dnd.is_some()
			&& !self.notification_preferences.settings.is_empty()
	}
	pub fn notification_allowed(&self, channel: Id) -> bool {
		self.notification_allowed_for(channel, true)
	}
	fn notification_allowed_for(&self, channel: Id, mention: bool) -> bool {
		if !self.gateway_connected
			|| !self.can_view(channel)
			|| !self.notification_preferences_known()
			|| self.notification_preferences.dnd == Some(true)
		{
			return false;
		}
		let Some(channel) = self
			.channels
			.iter()
			.find(|c| c.id == channel && c.supports_text())
		else {
			return false;
		};
		if channel.guild.is_some() && self.notification_preferences.dnd.is_none() {
			return false;
		}
		if channel.guild.is_none()
			&& (channel
				.recipients
				.iter()
				.any(|user| self.user_blocked(user.id) == Some(true))
				|| self.message_request_pending(channel.id)
				|| self.spam_direct(channel.id)
				|| self.pending_dm_muted(channel.id) == Some(true))
		{
			return false;
		}
		let Some(setting) = self.notification_preferences.settings.get(&channel.guild) else {
			// A missing global DM entry means default delivery, unless a known mute above applies.
			return channel.guild.is_none();
		};
		if setting.muted == Some(true) || (channel.guild.is_some() && setting.muted != Some(false))
		{
			return false;
		}
		let mut level = setting
			.level
			.or_else(|| channel.guild.is_none().then_some(0));
		for id in [channel.parent_id, Some(channel.id)].into_iter().flatten() {
			if let Some((_, muted, override_level)) =
				setting.channels.iter().find(|(c, ..)| *c == id)
			{
				let muted = if channel.guild.is_none() && id == channel.id {
					self.pending_dm_muted(id).or(*muted)
				} else if setting
					.channel_mute_until
					.iter()
					.any(|(channel, until)| *channel == id && *until <= Self::permission_time())
				{
					Some(false)
				} else {
					*muted
				};
				if muted == Some(true) || (channel.guild.is_some() && muted != Some(false)) {
					return false;
				}
				if override_level.is_some_and(|l| l != 3) {
					level = *override_level;
				}
			}
		}
		level == Some(0) || (mention && level == Some(1))
	}
	pub fn apply_notification_preferences(&mut self, event: Event) -> Result<(), &'static str> {
		self.observe_dm_settings(&event);
		self.read_state.activity.clear_notifications();
		if event.bytes() > MAX_SETTINGS_BYTES {
			self.notification_preferences = Preferences::default();
			return Err("Notification settings exceed safe byte capacity");
		}
		match event {
			Event::Presence(dnd) => {
				self.notification_preferences.dnd = dnd;
				if dnd.is_some() {
					self.startup_warnings.sessions = false;
				}
			}
			Event::Invalidate => self.notification_preferences = Preferences::default(),
			Event::Settings { entries, replace } => {
				let keys: BTreeSet<_> = entries.iter().map(|s| s.guild).collect();
				let old: Vec<_> = self
					.notification_preferences
					.settings
					.iter()
					.filter(|(key, _)| !replace && !keys.contains(key))
					.map(|(_, setting)| setting)
					.collect();
				let overrides = entries.iter().map(|s| s.channels.len()).sum::<usize>();
				let old_overrides: usize = old.iter().map(|s| s.channels.len()).sum();
				if old.len().saturating_add(entries.len()) > MAX_NAV
					|| old
						.iter()
						.map(|setting| setting.bytes() + 64)
						.sum::<usize>() + entries
						.iter()
						.map(|setting| setting.bytes() + 64)
						.sum::<usize>() > MAX_SETTINGS_BYTES
					|| overrides.saturating_add(old_overrides) > MAX_NAV
					|| keys.len() != entries.len()
					|| entries.iter().any(|s| {
						let channels: BTreeMap<_, _> = s
							.channels
							.iter()
							.map(|(id, muted, _)| (*id, *muted))
							.collect();
						if s.channel_mute_until.len() > s.channels.len()
							|| s.channel_mute_until
								.iter()
								.map(|(id, _)| *id)
								.collect::<BTreeSet<_>>()
								.len() != s.channel_mute_until.len()
							|| s.channel_mute_until.iter().any(|(id, until)| {
								*until < 0 || channels.get(id) != Some(&Some(true))
							}) {
							return true;
						}
						channels.len() != s.channels.len()
					}) {
					self.notification_preferences = Preferences::default();
					return Err(
						"Notification settings exceed safe capacity or contain duplicate IDs",
					);
				}
				if replace {
					self.notification_preferences.settings.clear();
					self.startup_warnings.notifications = false;
				}
				for setting in entries {
					self.notification_preferences
						.settings
						.insert(setting.guild, setting);
				}
			}
		}
		Ok(())
	}
	pub(crate) fn counts_toward_mention_badge(&self, message: &Message) -> bool {
		// Private recipient-remove rows are not mentions. Discord message type 2.
		if message.kind == 2
			|| !model::valid_mention_roles(&message.mention_roles)
			|| self
				.user
				.as_ref()
				.is_some_and(|owner| message.author.id == owner.id)
		{
			return false;
		}
		let direct = self
			.user
			.as_ref()
			.is_some_and(|owner| message.mentions.iter().any(|user| user.id == owner.id));
		self.mention_matches(
			message.channel,
			direct,
			message.mention_everyone,
			&message.mention_roles,
		)
	}
	fn mention_matches(&self, channel: Id, direct: bool, everyone: bool, roles: &[Id]) -> bool {
		let Some(channel) = self.channel(channel) else {
			return false;
		};
		if direct || channel.guild.is_none() {
			return true;
		}
		let Some(guild) = channel.guild else {
			return false;
		};
		let Some(setting) = self.notification_preferences.settings.get(&Some(guild)) else {
			return false;
		};
		(everyone && setting.suppress_everyone == Some(false))
			|| (setting.suppress_roles == Some(false)
				&& self
					.permissions
					.guilds
					.get(&guild)
					.and_then(|g| g.member.as_ref())
					.is_some_and(|member| roles.iter().any(|role| member.roles.contains(role))))
	}
	fn alert_preview(&self, message: &Message, mut text: &str) -> String {
		let guild = self
			.channel(message.channel)
			.and_then(|channel| channel.guild);
		let mut output = String::new();
		let mut count = 0;
		while !text.is_empty() && count < 160 && output.len() < 512 {
			let (prefix, name, len) = if let Some((id, len)) = model::user_mention_prefix(text) {
				let member = self
					.members
					.as_ref()
					.filter(|list| list.guild == guild)
					.and_then(|list| {
						list.rows
							.iter()
							.flatten()
							.find(|member| member.user.id == id)
					});
				let user = message
					.mentions
					.iter()
					.find(|user| user.id == id)
					.or_else(|| self.user.as_ref().filter(|user| user.id == id))
					.or_else(|| self.friend(id));
				let name = member
					.and_then(|member| member.nick.as_deref().filter(|nick| !nick.is_empty()))
					.or_else(|| user.map(|user| self.user_display_name(user)))
					.or_else(|| member.map(|member| member.user.name.as_str()))
					.unwrap_or("Unknown user");
				("@", name, len)
			} else if let Some((id, len)) = model::role_mention_prefix(text) {
				let name = guild
					.and_then(|guild| self.guild_roles(guild))
					.and_then(|roles| roles.iter().find(|role| role.id == id))
					.map_or("Unknown role", |role| role.name.as_str());
				("@", name, len)
			} else if let Some((id, len)) = model::channel_mention_prefix(text) {
				let name = self
					.channel(id)
					.filter(|_| self.can_view(id))
					.map_or("Unknown channel", |channel| channel.name.as_str());
				("#", name, len)
			} else {
				let len = text.chars().next().unwrap().len_utf8();
				("", &text[..len], len)
			};
			for character in prefix.chars().chain(name.chars()) {
				if count == 160 || output.len() + character.len_utf8() > 512 {
					return alert_text(&output, 160, 512);
				}
				output.push(character);
				count += 1;
			}
			text = &text[len..];
		}
		alert_text(&output, 160, 512)
	}

	pub(crate) fn observe_notification(&mut self, message: &Message) {
		if !model::valid_mention_roles(&message.mention_roles) {
			return;
		}
		if !self
			.channel(message.channel)
			.is_some_and(|channel| channel.supports_text())
		{
			return;
		}
		let activity = &mut self.read_state.activity;
		let previous = activity.high_water.get(&message.channel).copied();
		if previous.is_some_and(|id| message.id <= id) {
			return;
		}
		activity.high_water.insert(message.channel, message.id);
		if !self.can_view(message.channel) {
			return;
		}
		let Some(owner) = self.user.as_ref() else {
			return;
		};
		if message.author.id == owner.id {
			// The unofficial service also implicitly acknowledges our own gateway messages.
			let _ = self.apply_read_state(crate::read_state::Event::Ack {
				channel: message.channel,
				message: Some(message.id),
				manual: false,
				mention_count: Some(0),
				version: None,
			});
			return;
		}
		if self
			.read_marker(message.channel)
			.flatten()
			.is_some_and(|read| message.id <= read)
		{
			return;
		}
		let direct = message.mentions.iter().any(|u| u.id == owner.id);
		let mention = self.counts_toward_mention_badge(message);
		// Silent messages still contribute to badges, but never enqueue an OS alert.
		let allowed = !message.suppress_notifications
			&& self.user_blocked(message.author.id) != Some(true)
			&& self.notification_allowed_for(message.channel, mention);
		{
			let activity = &mut self.read_state.activity;
			// ponytail: retain 4096 observed messages; counts become a lower bound after eviction.
			while activity.observed.len() >= MAX_OBSERVED
				|| (activity.observed.len() + 1) * size_of::<(Id, Id, bool)>() > MAX_OBSERVED_BYTES
			{
				if let Some((channel, _, mention)) = activity.observed.pop_front()
					&& let Some(counts) = activity.observed_counts.get_mut(&channel)
				{
					counts.0 -= 1;
					counts.1 -= u32::from(mention);
					if counts.0 == 0 {
						activity.observed_counts.remove(&channel);
					}
				}
			}
			activity
				.observed
				.push_back((message.channel, message.id, mention));
			let counts = activity.observed_counts.entry(message.channel).or_default();
			counts.0 += 1;
			counts.1 += u32::from(mention);
		}
		if !allowed {
			return;
		}
		let sender = alert_text(self.message_author_name(message), 80, 256);
		let sender = if sender.is_empty() {
			"Unknown sender".to_owned()
		} else {
			sender
		};
		let display = message.display_text();
		let preview = if display.trim().is_empty() {
			if !message.attachments.is_empty() {
				"Sent an attachment".to_owned()
			} else if !message.embeds.is_empty() {
				"Sent an embed".to_owned()
			} else {
				"Sent a message".to_owned()
			}
		} else {
			let text = self.alert_preview(message, &display);
			if text.is_empty() {
				"Sent a message".to_owned()
			} else {
				text
			}
		};
		let notification = Notification {
			channel: message.channel,
			message: message.id,
			sender,
			preview,
			avatar_key: message.author.avatar_key(),
			direct,
			everyone: message.mention_everyone,
			roles: message.mention_roles.clone(),
		};
		let activity = &mut self.read_state.activity;
		while activity.notifications.len() >= MAX_NOTIFICATIONS
			|| activity
                .notifications
                .iter()
                .map(Notification::bytes)
                .sum::<usize>()
                + notification.bytes()
                // Include unused deque slots; capacity never exceeds the 32-item ceiling.
                + (MAX_NOTIFICATIONS - activity.notifications.len() - 1) * size_of::<Notification>()
				> MAX_NOTIFICATION_BYTES
		{
			activity.notifications.pop_front();
		}
		activity.notifications.push_back(notification);
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	use model::{Channel, Guild, Patch, User, permissions as p};
	fn notification_state() -> State {
		let owner = User {
			primary_guild: None,
			id: Id(2),
			name: "Synthetic".into(),
			avatar: None,
			webhook: false,
			kind: Default::default(),
			discriminator: 0,
		};
		let mut state = State {
			user: Some(owner.clone()),
			gateway_connected: true,
			auth: crate::auth::AuthState::Authenticated,
			guilds: vec![Guild {
				stickers: None,
				id: Id(1),
				name: "Synthetic".into(),
				icon: None,
				emojis: None,
			}],
			channels: [20, 21]
				.into_iter()
				.map(|id| Channel {
					id: Id(id),
					guild: Some(Id(1)),
					kind: 0,
					name: "Synthetic".into(),
					parent_id: None,
					last_message: Some(Id(95)),
					position: 0,
					recipients: vec![],
					icon: None,
					member_list_id: None,
					message_count: None,
				})
				.collect(),
			..State::default()
		};
		state
			.permissions
			.replace(p::Snapshot {
				guilds: vec![p::Guild {
					id: Id(1),
					owner: Some(Id(999)),
					roles: Some(vec![
						p::Role {
							id: Id(1),
							bits: p::VIEW_CHANNEL,
							name: String::new(),
							color: 0,
							position: 0,
							hoist: false,
						},
						p::Role {
							id: Id(10),
							bits: 0,
							name: String::new(),
							color: 0,
							position: 0,
							hoist: false,
						},
						p::Role {
							id: Id(11),
							bits: 0,
							name: String::new(),
							color: 0,
							position: 0,
							hoist: false,
						},
					]),
					member: Some(p::Member {
						roles: vec![],
						timeout_until: None,
					}),
				}],
				channels: [20, 21]
					.into_iter()
					.map(|id| p::Channel {
						id: Id(id),
						guild: Id(1),
						overwrites: Some(vec![]),
					})
					.collect(),
			})
			.unwrap();
		state
			.apply_read_state(crate::read_state::Event::Snapshot {
				entries: Some(vec![(Id(20), Some(Id(90)), 0), (Id(21), Some(Id(90)), 0)]),
				version: Some(1),
				partial: false,
			})
			.unwrap();
		state
			.apply_notification_preferences(Event::Settings {
				entries: vec![Setting {
					guild: Some(Id(1)),
					muted: Some(false),
					level: Some(0),
					suppress_everyone: Some(false),
					suppress_roles: Some(false),
					hide_muted_channels: None,
					channel_mute_until: vec![],
					channels: vec![],
				}],
				replace: true,
			})
			.unwrap();
		state
			.apply_notification_preferences(Event::Presence(Some(false)))
			.unwrap();
		state
	}
	#[test]
	fn confirmed_dm_mute_and_block_suppress_notifications() {
		let mut state = notification_state();
		let mut incoming = message(100, 20);
		incoming.author.id = Id(3);
		state.channels[0].guild = None;
		state.channels[0].kind = 1;
		state.channels[0].recipients = vec![incoming.author.clone()];
		state
			.apply_notification_preferences(Event::Settings {
				entries: vec![Setting {
					muted: Some(false),
					level: Some(0),
					..Default::default()
				}],
				replace: false,
			})
			.unwrap();
		state.observe_notification(&incoming);
		assert!(state.take_notification().is_some());
		let mute = state.set_dm_muted(Id(20), true).unwrap();
		assert_eq!(state.dm_muted(Id(20)), Some(true));
		assert!(!state.notification_allowed(Id(20)));
		state.command_rejected(mute);
		assert_eq!(state.dm_muted(Id(20)), Some(false));
		assert!(state.notification_allowed(Id(20)));
		state.confirm_dm_muted(Id(20), true).unwrap();
		let unmute = state.set_dm_muted(Id(20), false).unwrap();
		assert_eq!(state.dm_muted(Id(20)), Some(false));
		assert!(state.notification_allowed(Id(20)));
		state.command_rejected(unmute);
		assert!(!state.notification_allowed(Id(20)));
		incoming.id = Id(101);
		state.observe_notification(&incoming);
		assert!(state.take_notification().is_none());
		state.confirm_dm_muted(Id(20), false).unwrap();
		incoming.id = Id(102);
		state.observe_notification(&incoming);
		assert!(state.take_notification().is_some());
		state
			.apply_user_action(crate::user_actions::Event::Relationship {
				user: Id(3),
				blocked: true,
			})
			.unwrap();
		incoming.id = Id(103);
		state.observe_notification(&incoming);
		assert!(state.take_notification().is_none());
	}
	fn message(id: u64, channel: u64) -> Message {
		let owner = User {
			primary_guild: None,
			id: Id(2),
			name: "Synthetic".into(),
			avatar: None,
			webhook: false,
			kind: Default::default(),
			discriminator: 0,
		};
		Message {
			sticker_items: Vec::new(),
			kind: 0,
			id: Id(id),
			channel: Id(channel),
			author: User {
				id: Id(3),
				..owner.clone()
			},
			content: "Synthetic".into(),
			mentions: vec![owner.clone()],
			author_nick: None,
			author_roles: vec![],
			mention_roles: vec![],
			mention_everyone: false,
			suppress_notifications: false,
			reactions: Some(vec![]),
			edited: false,
			edited_at: None,
			revision: 0,
			nonce: None,
			reply_to: None,
			reply_deleted: false,
			forwarded: false,
			unsupported: false,
			components: vec![],
			application_id: None,
			flags: 0,
			ephemeral: false,
			extra_content: Default::default(),
			embeds: vec![],
			embeds_suppressed: false,
			attachments: vec![],
		}
	}
	#[test]
	fn incoming_alert_keeps_bounded_sender_preview_and_avatar_key() {
		let mut state = notification_state();
		let mut incoming = message(100, 20);
		incoming.author.name = "A sender\nname".into();
		incoming.author.avatar = Some("0123456789abcdef0123456789abcdef".into());
		incoming.content = "Hello\nfrom the call 🌍".repeat(100);
		state.observe_notification(&incoming);
		let alert = state.take_notification().expect("admitted incoming alert");
		assert_eq!(alert.sender, "A sender name");
		assert!(alert.preview.starts_with("Hello from the call 🌍"));
		assert!(alert.preview.len() <= 512);
		assert_eq!(alert.avatar_key, incoming.author.avatar_key());
		assert!(alert.bytes() <= MAX_NOTIFICATION_BYTES);
	}
	#[test]
	fn view_revocation_clears_alerts_and_badges_without_replaying_hidden_activity() {
		let mut state = notification_state();
		let access = |view| crate::permissions::Event::Channel {
			channel: Id(20),
			guild: Some(Id(1)),
			overwrites: Patch::Value(vec![p::Overwrite {
				id: Id(1),
				kind: 0,
				allow: 0,
				deny: if view { 0 } else { p::VIEW_CHANNEL },
			}]),
		};
		state.observe_notification(&message(100, 20));
		state.observe_notification(&message(101, 21));
		assert!(!state.can_read_history(Id(20)));
		assert_eq!(
			state.unread_count(Id(20)),
			1,
			"Live activity requires VIEW, not history access"
		);
		assert_eq!(state.mention_count(Id(20)), 1);
		assert_eq!(state.unread(Id(20)), Some(true));
		assert_eq!(state.channel_unread(&state.channels[0]), Some(true));
		state.apply(crate::Envelope {
			generation: state.generation,
			event: crate::Event::Permissions(access(false)),
		});
		assert_eq!(state.unread_count(Id(20)), 0);
		assert_eq!(state.mention_count(Id(20)), 0);
		assert_eq!(state.unread(Id(20)), None);
		assert_eq!(state.channel_unread(&state.channels[0]), None);
		assert_eq!(state.unread_count(Id(21)), 1);
		assert_eq!(
			state.read_state.activity.notifications.len(),
			1,
			"Revocation removes queued alerts before permission can return"
		);
		state.observe_notification(&message(102, 20));
		state.apply(crate::Envelope {
			generation: state.generation,
			event: crate::Event::Permissions(access(true)),
		});
		assert_eq!(state.take_notification().unwrap().channel, Id(21));
		state.observe_notification(&message(100, 20));
		state.observe_notification(&message(102, 20));
		assert_eq!(state.unread_count(Id(20)), 0);
		assert!(state.take_notification().is_none());
		state.observe_notification(&message(103, 20));
		assert_eq!(state.mention_count(Id(20)), 1);
		state.permissions.update(access(false)).unwrap();
		assert!(
			state.take_notification().is_none(),
			"Delivery independently checks current VIEW access"
		);
		state.permissions.update(access(true)).unwrap();
		state
			.apply_read_state(crate::read_state::Event::Ack {
				channel: Id(20),
				message: Some(Id(103)),
				manual: false,
				mention_count: Some(0),
				version: None,
			})
			.unwrap();
		state.observe_notification(&message(103, 20));
		assert_eq!(state.mention_count(Id(20)), 0);
		assert!(state.take_notification().is_none());
	}

	#[test]
	fn supplied_group_mentions_respect_membership_suppression_and_count_once() {
		for (direct, everyone, role, suppress_everyone, suppress_roles, expected) in [
			(false, false, true, Some(false), Some(false), true),
			(false, true, false, Some(false), Some(false), true),
			(true, true, true, Some(false), Some(false), true),
			(false, false, true, Some(false), Some(true), false),
			(false, true, false, Some(true), Some(false), false),
			(false, false, true, Some(false), None, false),
			(false, true, false, None, Some(false), false),
			(true, true, true, Some(true), Some(true), true),
		] {
			let mut state = notification_state();
			state
				.permissions
				.guilds
				.get_mut(&Id(1))
				.unwrap()
				.member
				.as_mut()
				.unwrap()
				.roles = vec![Id(10)];
			let setting = state
				.notification_preferences
				.settings
				.get_mut(&Some(Id(1)))
				.unwrap();
			setting.level = Some(1);
			setting.suppress_everyone = suppress_everyone;
			setting.suppress_roles = suppress_roles;
			let mut message = message(100, 20);
			if !direct {
				message.mentions.clear();
			}
			message.mention_everyone = everyone;
			if role {
				message.mention_roles = vec![Id(10), Id(11)];
			}
			state.observe_notification(&message);
			state.observe_notification(&message); // Replays cannot double-count or alert again.
			assert_eq!(state.unread_count(Id(20)), 1);
			assert_eq!(state.mention_count(Id(20)), u32::from(expected));
			assert_eq!(state.take_notification().is_some(), expected);
			assert!(state.take_notification().is_none());
		}
		for member_roles in [None, Some(vec![]), Some(vec![Id(11)])] {
			let mut state = notification_state();
			// Owner/admin permissions never imply membership in every role.
			let guild = state.permissions.guilds.get_mut(&Id(1)).unwrap();
			guild.owner = Some(Id(2));
			guild.member = member_roles.map(|roles| p::Member {
				roles,
				timeout_until: None,
			});
			let mut message = message(100, 20);
			message.mentions.clear();
			message.mention_roles = vec![Id(10)];
			state.observe_notification(&message);
			assert_eq!(state.mention_count(Id(20)), 0);
		}
	}
	#[test]
	fn silent_messages_keep_badges_and_never_queue_alerts_under_any_level() {
		for level in [0, 1] {
			let mut state = notification_state();
			state
				.notification_preferences
				.settings
				.get_mut(&Some(Id(1)))
				.unwrap()
				.level = Some(level);
			let mut message = message(100, 20);
			message.suppress_notifications = true;
			state.observe_notification(&message);
			assert_eq!(state.unread_count(Id(20)), 1);
			assert_eq!(state.mention_count(Id(20)), 1);
			assert!(state.take_notification().is_none());
			// Suppressed group pings are still ordinary messages at all-messages level.
			let setting = state
				.notification_preferences
				.settings
				.get_mut(&Some(Id(1)))
				.unwrap();
			setting.suppress_everyone = Some(true);
			message.id = Id(101);
			message.suppress_notifications = false;
			message.mentions.clear();
			message.mention_everyone = true;
			state.observe_notification(&message);
			assert_eq!(state.mention_count(Id(20)), 1);
			assert_eq!(state.take_notification().is_some(), level == 0);
		}
	}
	#[test]
	fn queued_role_alerts_recheck_membership_and_payload_capacity_at_delivery() {
		let mut state = notification_state();
		state
			.notification_preferences
			.settings
			.get_mut(&Some(Id(1)))
			.unwrap()
			.level = Some(1);
		state
			.permissions
			.guilds
			.get_mut(&Id(1))
			.unwrap()
			.member
			.as_mut()
			.unwrap()
			.roles = vec![Id(10), Id(11)];
		let mut message = message(100, 20);
		message.mentions.clear();
		message.mention_roles = vec![Id(10), Id(11)];
		state.observe_notification(&message);
		state
			.permissions
			.update(crate::permissions::Event::Member {
				guild: Id(1),
				roles: Patch::Value(vec![Id(11)]),
				timeout_until: Patch::Absent,
			})
			.unwrap();
		assert!(
			state.take_notification().is_some(),
			"A second matching role still qualifies"
		);
		message.id = Id(101);
		state.observe_notification(&message);
		state
			.permissions
			.update(crate::permissions::Event::Member {
				guild: Id(1),
				roles: Patch::Value(vec![]),
				timeout_until: Patch::Absent,
			})
			.unwrap();
		assert!(state.take_notification().is_none());
		assert_eq!(
			state.mention_count(Id(20)),
			2,
			"Delivery revalidation does not rewrite observed history"
		);
		state
			.notification_preferences
			.settings
			.get_mut(&Some(Id(1)))
			.unwrap()
			.level = Some(0);
		message.mention_roles = (10..110).map(Id).collect();
		for id in 102..202 {
			message.id = Id(id);
			state.observe_notification(&message);
		}
		let queue = &state.read_state.activity.notifications;
		assert!(
			queue.len() < MAX_NOTIFICATIONS,
			"Payload budget evicts before item ceiling"
		);
		assert!(
			queue.iter().map(Notification::bytes).sum::<usize>()
				+ (queue.capacity() - queue.len()) * size_of::<Notification>()
				<= MAX_NOTIFICATION_BYTES
		);
		state
			.apply_notification_preferences(Event::Invalidate)
			.unwrap();
		assert!(state.take_notification().is_none());
	}
	#[test]
	fn unknown_deletes_and_empty_recounts_never_admit_channel_state() {
		let mut activity = Activity::default();
		for id in 1..=10_000 {
			activity.delete(Id(id), Id(id));
			activity.recount(Id(id));
		}
		assert!(activity.observed_counts.is_empty());
		assert!(activity.high_water.is_empty());
		assert!(activity.counts.is_empty());
		activity.observed.push_back((Id(1), Id(2), true));
		activity.recount(Id(1));
		assert_eq!(activity.count(Id(1), true), 1);
		activity.delete(Id(1), Id(2));
		assert!(activity.observed_counts.is_empty());
		assert!(activity.observed.is_empty());
	}
	#[test]
	fn marking_unread_badges_guild_mentions_inside_the_new_range() {
		let mut state = notification_state();
		state.freshness = model::Freshness::Fresh;
		state.selected = Some(Id(20));
		state
			.channels
			.iter_mut()
			.find(|channel| channel.id == Id(20))
			.unwrap()
			.last_message = Some(Id(100));
		state
			.apply_read_state(crate::read_state::Event::Ack {
				channel: Id(20),
				message: Some(Id(100)),
				manual: false,
				mention_count: Some(0),
				version: Some(2),
			})
			.unwrap();
		let mut plain = message(96, 20);
		plain.mentions.clear();
		let ping = message(97, 20);
		let mut mine = message(98, 20);
		mine.author.id = Id(2);
		mine.mentions.clear();
		let mut everyone = message(99, 20);
		everyone.mentions.clear();
		everyone.mention_everyone = true;
		let mut later = message(100, 20);
		later.mentions.clear();
		for row in [plain, ping, mine, everyone, later] {
			state.timeline.insert(row, false, false).unwrap();
		}
		assert_eq!(state.mention_count(Id(20)), 0);
		assert_eq!(state.unread(Id(20)), Some(false));
		let crate::Command::MarkRead {
			channel,
			message,
			request,
			manual: true,
			mention_count,
		} = state.prepare_mark_unread(Id(96)).unwrap()
		else {
			panic!("mark unread");
		};
		assert_eq!(message, Id(95));
		assert_eq!(mention_count, Some(2));
		state
			.apply_read_state(crate::read_state::Event::Result {
				channel,
				message,
				request,
				result: Ok(()),
			})
			.unwrap();
		assert_eq!(state.unread(Id(20)), Some(true));
		assert_eq!(state.mention_count(Id(20)), 2);
		state
			.apply_read_state(crate::read_state::Event::Ack {
				channel,
				message: Some(message),
				manual: true,
				mention_count: None,
				version: Some(3),
			})
			.unwrap();
		assert_eq!(state.mention_count(Id(20)), 2);
		assert_eq!(state.unread(Id(20)), Some(true));
	}
}
