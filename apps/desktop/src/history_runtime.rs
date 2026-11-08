//! Native persistent-history lifecycle. No Discord requests or extension data are introduced.
use crate::{
	cache,
	history_store::{ExportRequest, ViewPage},
	message_history_export::{ExportError, ExportSummary},
};
use client_core::{Command, State};
use local_store::{
	StoreError,
	message_history::{HistoryAttachmentUpdate, HistoryReservation, HistorySettings},
};
use model::{
	Id,
	message_logger::{DownloadRequest, Mutation},
};
use std::{
	collections::{BTreeMap, VecDeque},
	path::PathBuf,
	sync::{
		Arc,
		atomic::{AtomicBool, Ordering},
	},
};
use ui::message_history::{ClearScope, ExportFormat, Preferences, Request};

fn parse_filters(value: &str) -> Result<Vec<Id>, &'static str> {
	if value.len() > ui::message_history::MAX_PREFERENCE_LIST_CHARS {
		return Err("Ignore list is too long");
	}
	let mut ids = std::collections::BTreeSet::new();
	for value in value.split(|character: char| character == ',' || character.is_whitespace()) {
		if value.is_empty() {
			continue;
		}
		let id = value
			.parse::<u64>()
			.map_err(|_| "Ignore lists accept only decimal Discord IDs")?;
		if id == 0 || !ids.insert(Id(id)) {
			return Err("Ignore list IDs must be nonzero and unique");
		}
		if ids.len() > 512 {
			return Err("Each ignore list may contain at most 512 IDs");
		}
	}
	Ok(ids.into_iter().collect())
}

fn join_filters(ids: &[Id]) -> String {
	ids.iter().map(Id::to_string).collect::<Vec<_>>().join(",")
}

#[derive(Default)]
pub struct TickEffects {
	pub allowed: bool,
	pub commands: Vec<Command>,
	pub cache_operations: usize,
	pub storage_failed: bool,
}

struct ExportSession {
	format: ExportFormat,
	cancelled: Arc<AtomicBool>,
	picker: Option<std::sync::mpsc::Receiver<Option<PathBuf>>>,
}

struct MediaTicket {
	account: Id,
	channel: Id,
	message: Id,
	attachment: Id,
	request: Option<DownloadRequest>,
	job: Option<u64>,
}

#[derive(Default)]
pub struct Runtime {
	scope: Option<(u64, Id)>,
	settings: HistorySettings,
	configured: bool,
	settings_loaded: bool,
	settings_restored: bool,
	settings_dirty: bool,
	settings_saving: bool,
	activation: bool,
	pending: VecDeque<Mutation>,
	pending_bytes: usize,
	export: Option<ExportSession>,
	storage_warning: Option<&'static str>,
	last_retention: Option<std::time::Instant>,
	media_cleanup: VecDeque<std::sync::mpsc::Receiver<(Id, Result<(), &'static str>)>>,
	media: Option<Arc<crate::message_history_media::MediaManager>>,
	media_tickets: BTreeMap<u64, MediaTicket>,
	media_next: u64,
}

impl Runtime {
	pub fn capture_allowed(&self) -> bool {
		self.settings_restored && self.activation && self.settings.enabled
	}

	pub fn tick(
		&mut self,
		state: &mut State,
		messaging: &mut ui::MessagingUi,
		cache: Option<&cache::Cache>,
		runtime: &tokio::runtime::Runtime,
		window: &Arc<winit::window::Window>,
		activated: bool,
	) -> TickEffects {
		let mut effects = TickEffects::default();
		let account = state.user.as_ref().map(|user| user.id);
		let scope = account.map(|account| (state.generation, account));
		self.poll_media_cleanup(messaging);
		if self.scope != scope {
			self.reset_scope(scope, messaging, cache, &mut effects);
		}
		if let Some((_, account)) = scope {
			if !self.settings_loaded
				&& let Some(cache) = cache
			{
				self.settings_loaded = true;
				if cache.queue(
					state.generation,
					account,
					cache::Operation::LoadHistorySettings,
				) {
					effects.cache_operations += 1;
				} else {
					self.settings_loaded = false;
					effects.storage_failed = true;
				}
			}
			messaging.message_history_mut().set_capture_controls(
				state.generation,
				Some(account),
				activated,
			);
			self.observe_activation(activated);
			self.sync_policy(state);
			self.flush_settings(state.generation, account, cache, &mut effects);
			self.flush_mutations(state, account, cache, &mut effects);
			self.poll_export(state.generation, account, messaging, cache, &mut effects);
			self.poll_retention(state.generation, account, cache, &mut effects);
			self.requests(state, messaging, cache, runtime, window, &mut effects);
			self.queue_media(state, account, cache, messaging, &mut effects);
			self.poll_media(state, account, cache, messaging, &mut effects);
		}
		effects.allowed = self.capture_allowed();
		effects
	}

	fn reset_scope(
		&mut self,
		scope: Option<(u64, Id)>,
		messaging: &mut ui::MessagingUi,
		cache: Option<&cache::Cache>,
		effects: &mut TickEffects,
	) {
		self.cancel_media(cache, effects);
		if let Some(export) = self.export.take() {
			export.cancelled.store(true, Ordering::Release);
		}
		if let Some(cache) = cache {
			// Old non-destructive history operations lose their fence; explicit forget still runs.
			cache.history.invalidate();
		}
		let (generation, account) = scope.map_or((0, None), |(generation, account)| {
			(generation, Some(account))
		});
		messaging
			.message_history_mut()
			.sync_scope(generation, account);
		self.scope = scope;
		self.settings = HistorySettings::default();
		self.configured = false;
		self.settings_loaded = false;
		self.settings_restored = false;
		self.settings_dirty = false;
		self.settings_saving = false;
		self.activation = false;
		self.pending.clear();
		self.pending_bytes = 0;
		self.storage_warning = None;
		self.last_retention = None;
	}

	fn queue_media(
		&mut self,
		state: &mut State,
		account: Id,
		cache: Option<&cache::Cache>,
		messaging: &mut ui::MessagingUi,
		effects: &mut TickEffects,
	) {
		let Some(cache) = cache else { return };
		for request in state.message_logger.drain_download_requests() {
			let mime = request
				.attachment
				.content_type
				.as_deref()
				.unwrap_or("")
				.split(';')
				.next()
				.unwrap_or("");
			let allowed = match mime {
				"image/png" | "image/jpeg" | "image/gif" | "image/webp" => {
					self.settings.downloads.images
				}
				"audio/mpeg" | "audio/ogg" | "audio/wav" => self.settings.downloads.audio,
				"video/mp4" | "video/webm" => self.settings.downloads.video,
				_ => false,
			};
			if !allowed {
				continue;
			}
			if self.media_tickets.len() >= crate::message_history_media::MAX_JOBS {
				messaging.toasts.push(
					ui::design::Level::Warning,
					"Attachment download queue is full; this attachment remains metadata-only.",
				);
				continue;
			}
			self.media_next = self.media_next.wrapping_add(1).max(1);
			let ticket = self.media_next;
			let (channel, message, attachment) =
				(request.channel, request.message, request.attachment.id);
			self.media_tickets.insert(
				ticket,
				MediaTicket {
					account,
					channel,
					message,
					attachment,
					request: Some(request.clone()),
					job: None,
				},
			);
			if !cache.queue(
				state.generation,
				account,
				cache::Operation::ReserveHistoryAttachment {
					request: ticket,
					channel,
					message,
					attachment,
					expected_bytes: request.attachment.size,
				},
			) {
				self.media_tickets.remove(&ticket);
				messaging.toasts.push(
					ui::design::Level::Warning,
					"Could not reserve attachment storage; the download was not started.",
				);
				effects.storage_failed = true;
				continue;
			}
			effects.cache_operations += 1;
		}
	}

	fn finish_media(
		&self,
		generation: u64,
		ticket: &MediaTicket,
		actual: Result<u64, &'static str>,
		cache: Option<&cache::Cache>,
		effects: &mut TickEffects,
	) {
		if let Some(cache) = cache
			&& cache.queue(
				generation,
				ticket.account,
				cache::Operation::FinishHistoryAttachment {
					channel: ticket.channel,
					message: ticket.message,
					attachment: ticket.attachment,
					actual,
				},
			) {
			effects.cache_operations += 1;
		} else {
			effects.storage_failed = true;
		}
	}

	fn cancel_media(&mut self, cache: Option<&cache::Cache>, effects: &mut TickEffects) {
		let Some((generation, account)) = self.scope else {
			self.media_tickets.clear();
			return;
		};
		if let Some(media) = &self.media {
			media.cancel_account(account.0);
		}
		for ticket in self.media_tickets.values() {
			self.finish_media(generation, ticket, Err("Cancelled"), cache, effects);
		}
		self.media_tickets.clear();
	}

	fn manager(
		&mut self,
		cache: Option<&cache::Cache>,
	) -> Result<Arc<crate::message_history_media::MediaManager>, &'static str> {
		if let Some(manager) = &self.media {
			return Ok(manager.clone());
		}
		let manager = if let Some(cache) = cache {
			cache
				.media
				.manager()
				.map_err(|_| "Media worker unavailable")?
		} else {
			let root =
				local_store::data_dir().map_err(|_| "Application data directory is unavailable")?;
			Arc::new(crate::message_history_media::MediaManager::new(
				crate::message_history_media::MediaConfig::new(root),
			)?)
		};
		self.media = Some(manager.clone());
		Ok(manager)
	}

	pub fn accept_reservation(
		&mut self,
		serial: u64,
		result: Result<HistoryReservation, StoreError>,
		state: &State,
		messaging: &mut ui::MessagingUi,
		cache: Option<&cache::Cache>,
	) -> TickEffects {
		let mut effects = TickEffects::default();
		let Some(mut ticket) = self.media_tickets.remove(&serial) else {
			return effects;
		};
		if self.scope != Some((state.generation, ticket.account)) {
			return effects;
		}
		match result {
			Ok(HistoryReservation::Reserved { .. } | HistoryReservation::AlreadyReserved) => {
				let Some(request) = ticket.request.take() else {
					return effects;
				};
				let manager = match self.manager(cache) {
					Ok(manager) => manager,
					Err(error) => {
						messaging.toasts.push(ui::design::Level::Warning, error);
						self.finish_media(
							state.generation,
							&ticket,
							Err("Media worker unavailable"),
							cache,
							&mut effects,
						);
						return effects;
					}
				};
				let converted = crate::message_history_media::MediaRequest {
					account: ticket.account.0,
					channel: ticket.channel.0,
					message: Some(ticket.message.0),
					attachment: ticket.attachment.0,
					filename: request.attachment.filename,
					content_type: request.attachment.content_type,
					size: request.attachment.size,
					url: request.url,
				};
				match manager.enqueue(converted) {
					Ok(job) => {
						ticket.job = Some(job);
						self.media_tickets.insert(serial, ticket);
					}
					Err(error) => {
						messaging.toasts.push(ui::design::Level::Warning, error);
						self.finish_media(
							state.generation,
							&ticket,
							Err(error),
							cache,
							&mut effects,
						);
					}
				}
			}
			Ok(HistoryReservation::AlreadyStored) => {
				messaging.message_history_mut().set_status(
					state.generation,
					"This attachment was already retained locally.",
					None,
					true,
				);
			}
			Ok(HistoryReservation::Disabled) => {
				messaging.toasts.push(
					ui::design::Level::Info,
					"This attachment type is not enabled for history downloads.",
				);
			}
			Ok(HistoryReservation::TooLarge) => {
				messaging.toasts.push(
					ui::design::Level::Warning,
					"Attachment exceeds the history download limit.",
				);
			}
			Ok(HistoryReservation::AccountFull) => {
				messaging.toasts.push(
					ui::design::Level::Warning,
					"The account's 256 MiB history media budget is full.",
				);
			}
			Ok(HistoryReservation::Unknown) => {
				messaging.toasts.push(
					ui::design::Level::Info,
					"The attachment is no longer in retained history; no download was started.",
				);
			}
			Err(_) => {
				messaging.toasts.push(
					ui::design::Level::Warning,
					"Could not reserve attachment storage; the download was not started.",
				);
				self.finish_media(
					state.generation,
					&ticket,
					Err("Reservation failed"),
					cache,
					&mut effects,
				);
				effects.storage_failed = true;
			}
		}
		effects
	}

	fn poll_media(
		&mut self,
		state: &State,
		account: Id,
		cache: Option<&cache::Cache>,
		messaging: &mut ui::MessagingUi,
		effects: &mut TickEffects,
	) {
		let Some(manager) = self.media.clone() else {
			return;
		};
		if let Some(progress) = manager.progress()
			&& messaging.message_history().is_open()
		{
			messaging.message_history_mut().set_status(
				state.generation,
				format!(
					"Downloading retained attachment: {} of {} bytes",
					progress.received, progress.total
				),
				None,
				false,
			);
		} else if manager.queued() > 0 && messaging.message_history().is_open() {
			messaging.message_history_mut().set_status(
				state.generation,
				format!(
					"{} retained attachment downloads are waiting.",
					manager.queued()
				),
				None,
				false,
			);
		}
		for completion in manager.poll() {
			let Some((serial, _)) = self
				.media_tickets
				.iter()
				.find(|(_, ticket)| ticket.job == Some(completion.job))
				.map(|(serial, ticket)| (*serial, ticket.account))
			else {
				continue;
			};
			let Some(ticket) = self.media_tickets.remove(&serial) else {
				continue;
			};
			if ticket.account != account {
				continue;
			}
			let actual = match completion.outcome {
				crate::message_history_media::MediaOutcome::Saved { bytes, .. } => Ok(bytes),
				crate::message_history_media::MediaOutcome::Cancelled => Err("Cancelled"),
				crate::message_history_media::MediaOutcome::Failed(error) => Err(error),
			};
			self.finish_media(state.generation, &ticket, actual, cache, effects);
		}
	}

	pub fn accept_media_finished(
		&mut self,
		generation: u64,
		result: Result<HistoryAttachmentUpdate, StoreError>,
		messaging: &mut ui::MessagingUi,
	) {
		let message = match result {
			Ok(HistoryAttachmentUpdate::Stored) => {
				"Attachment bytes and local accounting were saved."
			}
			Ok(HistoryAttachmentUpdate::Released) => {
				"Attachment reservation released; metadata remains bounded."
			}
			Ok(HistoryAttachmentUpdate::TooLarge) => {
				"Downloaded attachment exceeded the storage limit."
			}
			Ok(HistoryAttachmentUpdate::AccountFull) => {
				"The account media budget filled before completion."
			}
			Ok(HistoryAttachmentUpdate::NotReserved | HistoryAttachmentUpdate::Unknown) => {
				"Attachment metadata changed before its reservation completed."
			}
			Err(_) => "Could not record the attachment download outcome.",
		};
		messaging
			.message_history_mut()
			.set_status(generation, message, None, true);
	}

	fn poll_media_cleanup(&mut self, messaging: &mut ui::MessagingUi) {
		for _ in 0..4 {
			let Some(receiver) = self.media_cleanup.pop_front() else {
				break;
			};
			match receiver.try_recv() {
				Ok((_, Ok(()))) => {}
				Ok((account, Err(error))) => messaging.toasts.push(
					ui::design::Level::Warning,
					format!("Account {account} attachment cleanup failed: {error}. Remaining files require another cleanup attempt."),
				),
				Err(std::sync::mpsc::TryRecvError::Empty) => self.media_cleanup.push_back(receiver),
				Err(std::sync::mpsc::TryRecvError::Disconnected) => messaging.toasts.push(
					ui::design::Level::Warning,
					"Attachment cleanup stopped unexpectedly; remaining files require another cleanup attempt.",
				),
			}
		}
	}

	fn observe_activation(&mut self, activated: bool) {
		if !self.settings_restored {
			return;
		}
		if activated && !self.activation && !self.configured {
			self.settings.enabled = true;
			self.settings_dirty = true;
		}
		self.activation = activated;
	}

	fn apply_preferences(
		&mut self,
		generation: u64,
		values: Preferences,
		messaging: &mut ui::MessagingUi,
	) {
		if !self.activation || !values.enabled {
			messaging.message_history_mut().set_status(
				generation,
				"Enable and activate the reviewed Message History package first. Disabling uses Clear.",
				None,
				true,
			);
			return;
		}
		let users = match parse_filters(&values.ignore_users) {
			Ok(users) => users,
			Err(error) => {
				messaging
					.message_history_mut()
					.set_status(generation, error, None, true);
				return;
			}
		};
		let channels = match parse_filters(&values.ignore_channels) {
			Ok(channels) => channels,
			Err(error) => {
				messaging
					.message_history_mut()
					.set_status(generation, error, None, true);
				return;
			}
		};
		let retention_days = values.retention_days;
		let max_messages = values.max_messages;
		let mut next = self.settings.clone();
		next.enabled = true;
		next.retention_days = retention_days;
		next.max_messages = max_messages;
		next.alert_mode = match values.ghost {
			model::message_logger::GhostMode::Off => {
				local_store::message_history::HistoryAlertMode::Off
			}
			model::message_logger::GhostMode::Mentions => {
				local_store::message_history::HistoryAlertMode::Mentions
			}
			model::message_logger::GhostMode::All => {
				local_store::message_history::HistoryAlertMode::All
			}
		};
		next.downloads.images = values.download_images;
		next.downloads.audio = values.download_audio;
		next.downloads.video = values.download_video;
		next.ignore_users = users;
		next.ignore_channels = channels;
		if !next.is_valid() {
			messaging.message_history_mut().set_status(
				generation,
				"History retention values or ignore filters are outside their supported bounds.",
				None,
				true,
			);
			return;
		}
		self.settings = next;
		self.settings_dirty = true;
		self.configured = true;
		messaging.message_history_mut().set_status(
			generation,
			"Saving bounded history preferences…",
			None,
			false,
		);
	}

	fn sync_policy(&self, state: &mut State) {
		state
			.message_logger
			.set_ghost_mode(crate::history_store::ghost_mode(self.settings.alert_mode));
		state
			.message_logger
			.set_ignored_users(self.settings.ignore_users.iter().copied());
		state
			.message_logger
			.set_ignored_channels(self.settings.ignore_channels.iter().copied());
		state.message_logger.set_attachment_downloads(
			self.settings.downloads.images
				|| self.settings.downloads.audio
				|| self.settings.downloads.video,
		);
	}

	fn preferences(&self) -> Preferences {
		Preferences {
			enabled: self.settings.enabled,
			retention_days: self.settings.retention_days,
			max_messages: self.settings.max_messages,
			ghost: match self.settings.alert_mode {
				local_store::message_history::HistoryAlertMode::Off => {
					model::message_logger::GhostMode::Off
				}
				local_store::message_history::HistoryAlertMode::Mentions => {
					model::message_logger::GhostMode::Mentions
				}
				local_store::message_history::HistoryAlertMode::All => {
					model::message_logger::GhostMode::All
				}
			},
			download_images: self.settings.downloads.images,
			download_audio: self.settings.downloads.audio,
			download_video: self.settings.downloads.video,
			ignore_users: join_filters(&self.settings.ignore_users),
			ignore_channels: join_filters(&self.settings.ignore_channels),
		}
	}

	fn sync_preferences(&self, generation: u64, account: Id, messaging: &mut ui::MessagingUi) {
		messaging.message_history_mut().set_preferences(
			generation,
			Some(account),
			self.preferences(),
		);
	}

	fn flush_settings(
		&mut self,
		generation: u64,
		account: Id,
		cache: Option<&cache::Cache>,
		effects: &mut TickEffects,
	) {
		if !self.settings_dirty || self.settings_saving {
			return;
		}
		let Some(cache) = cache else { return };
		if cache.queue(
			generation,
			account,
			cache::Operation::SaveHistorySettings(self.settings.clone()),
		) {
			self.settings_dirty = false;
			self.settings_saving = true;
			effects.cache_operations += 1;
		} else {
			effects.storage_failed = true;
		}
	}

	fn flush_mutations(
		&mut self,
		state: &mut State,
		account: Id,
		cache: Option<&cache::Cache>,
		effects: &mut TickEffects,
	) {
		for mutation in state.message_logger.drain_mutations() {
			self.pending_bytes = self.pending_bytes.saturating_add(mutation.bytes());
			self.pending.push_back(mutation);
		}
		while self.pending.len() > model::message_logger::MAX_MUTATIONS
			|| self.pending_bytes > model::message_logger::MAX_MUTATION_BYTES
		{
			if let Some(old) = self.pending.pop_front() {
				self.pending_bytes = self.pending_bytes.saturating_sub(old.bytes());
				self.storage_warning =
					Some("History queue is full; oldest unsaved observations were dropped");
			}
		}
		let Some(cache) = cache else { return };
		for _ in 0..8 {
			let epoch = state.message_logger.epoch();
			let mut batch = Vec::new();
			let mut bytes = 0usize;
			for mutation in self.pending.iter().take(64) {
				if mutation.account() != account {
					continue;
				}
				let destructive = !matches!(
					mutation,
					Mutation::Baseline { .. } | Mutation::Edit { .. } | Mutation::Delete { .. }
				);
				if !destructive && mutation.epoch() != epoch {
					continue;
				}
				let next = bytes.saturating_add(mutation.bytes());
				if !batch.is_empty() && next > 256 * 1024 {
					break;
				}
				bytes = next;
				batch.push(mutation.clone());
			}
			if batch.is_empty() {
				break;
			}
			if !cache.queue(
				state.generation,
				account,
				cache::Operation::ApplyHistoryMutations(batch.clone()),
			) {
				effects.storage_failed = true;
				break;
			}
			effects.cache_operations += 1;
			let mut sent = 0usize;
			while sent < batch.len() {
				let Some(mutation) = self.pending.pop_front() else {
					break;
				};
				self.pending_bytes = self.pending_bytes.saturating_sub(mutation.bytes());
				if batch.get(sent) == Some(&mutation) {
					sent += 1;
				}
			}
		}
	}

	fn poll_retention(
		&mut self,
		generation: u64,
		account: Id,
		cache: Option<&cache::Cache>,
		effects: &mut TickEffects,
	) {
		let now = std::time::Instant::now();
		if self
			.last_retention
			.is_some_and(|last| now.duration_since(last) < std::time::Duration::from_secs(600))
		{
			return;
		}
		let Some(cache) = cache else { return };
		let now_ms = std::time::SystemTime::now()
			.duration_since(std::time::UNIX_EPOCH)
			.map_or(0, |duration| {
				duration.as_millis().min(i64::MAX as u128) as i64
			});
		if cache.queue(
			generation,
			account,
			cache::Operation::MaintainHistoryLog { now_ms },
		) {
			self.last_retention = Some(now);
			effects.cache_operations += 1;
		}
	}

	fn requests(
		&mut self,
		state: &mut State,
		messaging: &mut ui::MessagingUi,
		cache: Option<&cache::Cache>,
		runtime: &tokio::runtime::Runtime,
		window: &Arc<winit::window::Window>,
		effects: &mut TickEffects,
	) {
		for request in messaging.take_message_history_requests() {
			match request {
				Request::LoadPage {
					generation,
					account: Some(account),
					channel,
					author,
					message,
					kind,
					before,
					after,
					limit,
				} if generation == state.generation && limit > 0 && limit <= 100 => {
					let request = ViewPage {
						generation,
						account,
						channel,
						author,
						message,
						kind,
						before: before.map(|key| (key.channel, key.message)),
						after: after.map(|key| (key.channel, key.message)),
						limit: limit as u32,
					};
					if let Some(cache) = cache
						&& cache.queue(
							generation,
							account,
							cache::Operation::LoadHistoryPage(request),
						) {
						effects.cache_operations += 1;
					} else {
						messaging.message_history_mut().page_failed(
							generation,
							"Local history storage is unavailable; the page was not loaded.",
						);
					}
				}
				Request::LoadPage { generation, .. } => {
					messaging.message_history_mut().page_failed(
						generation,
						"Sign in to an account before opening retained history.",
					);
				}
				Request::SavePreferences {
					generation,
					account: Some(_),
					preferences,
				} if generation == state.generation => {
					self.apply_preferences(generation, preferences, messaging);
				}
				Request::OpenTarget {
					generation,
					channel,
					message,
					deleted,
				} if generation == state.generation => {
					if deleted {
						let retained = state.message_logger.get(channel, message).cloned();
						messaging.message_history_mut().open_target(
							generation,
							state.user.as_ref().map(|user| user.id),
							channel,
							message,
							retained,
						);
						if let Some(command) = state.select(channel) {
							effects.commands.push(command);
						}
					} else if let Some(command) = state.open_message_target(channel, message) {
						effects.commands.push(command);
					}
				}
				Request::Clear {
					generation,
					account: Some(account),
					scope,
				} if generation == state.generation => {
					let channel = match scope {
						ClearScope::Channel(channel) => Some(channel),
						ClearScope::Account => None,
					};
					if let Some(cache) = cache
						&& cache.queue(
							generation,
							account,
							cache::Operation::ClearHistoryLog { channel },
						) {
						effects.cache_operations += 1;
						if channel.is_none() {
							self.settings.enabled = false;
							self.configured = true;
							state.message_logger.revoke_persistent();
						} else if let Some(channel) = channel {
							state.message_logger.revoke_channel(channel);
						}
					} else {
						messaging.message_history_mut().set_status(
							generation,
							"Could not queue history clearing; retained rows were not cleared.",
							None,
							true,
						);
						effects.storage_failed = true;
					}
				}
				Request::Export {
					generation,
					account: Some(account),
					format,
				} if generation == state.generation => {
					self.begin_export(generation, account, format, messaging, runtime, window);
				}
				Request::Clear { .. }
				| Request::Export { .. }
				| Request::SavePreferences { .. }
				| Request::OpenTarget { .. } => {}
			}
		}
		if let Some(warning) = self.storage_warning.take() {
			messaging.toasts.push(ui::design::Level::Warning, warning);
		}
	}

	fn begin_export(
		&mut self,
		generation: u64,
		account: Id,
		format: ExportFormat,
		messaging: &mut ui::MessagingUi,
		runtime: &tokio::runtime::Runtime,
		window: &Arc<winit::window::Window>,
	) {
		if self.export.is_some() {
			messaging.message_history_mut().set_status(
				generation,
				"Another history export is already being prepared.",
				None,
				false,
			);
			return;
		}
		let (send, receive) = std::sync::mpsc::sync_channel(1);
		let format_key = match format {
			ExportFormat::Text => "txt",
			ExportFormat::Json => "json",
		};
		let future = platform::save::message_history_destination(window.clone(), format_key);
		runtime.spawn(async move {
			let _ = send.send(future.await);
		});
		self.export = Some(ExportSession {
			format,
			cancelled: Arc::new(AtomicBool::new(false)),
			picker: Some(receive),
		});
		messaging.message_history_mut().set_status(
			generation,
			format!("Choose a {format_key} destination for account {account}'s history export."),
			None,
			false,
		);
	}

	fn poll_export(
		&mut self,
		generation: u64,
		account: Id,
		messaging: &mut ui::MessagingUi,
		cache: Option<&cache::Cache>,
		effects: &mut TickEffects,
	) {
		let Some(session) = &mut self.export else {
			return;
		};
		let result = session
			.picker
			.as_ref()
			.and_then(|picker| picker.try_recv().ok());
		let Some(destination) = result else { return };
		let Some(destination) = destination else {
			messaging.message_history_mut().set_status(
				generation,
				"History export cancelled; no file was written.",
				None,
				true,
			);
			self.export = None;
			return;
		};
		let format = session.format;
		let request = ExportRequest {
			destination,
			format,
			cancelled: session.cancelled.clone(),
		};
		if let Some(cache) = cache
			&& cache.queue(
				generation,
				account,
				cache::Operation::ExportHistory(request),
			) {
			effects.cache_operations += 1;
			if let Some(session) = &mut self.export {
				session.picker = None;
			}
			messaging.message_history_mut().set_status(
				generation,
				"Exporting the account's retained rows with bounded row and byte limits…",
				None,
				false,
			);
		} else {
			messaging.message_history_mut().set_status(
				generation,
				"Could not queue the history export; no file was written.",
				None,
				true,
			);
			self.export = None;
			effects.storage_failed = true;
		}
	}

	pub fn accept_settings(
		&mut self,
		result: Result<(HistorySettings, bool), StoreError>,
		state: &mut State,
		messaging: &mut ui::MessagingUi,
	) {
		match result {
			Ok((settings, configured)) => {
				self.settings = settings;
				self.settings_restored = true;
				self.configured = configured;
				self.activation = false;
				self.sync_policy(state);
			}
			Err(_) => {
				self.settings_loaded = false;
				self.settings_restored = false;
				self.settings.enabled = false;
				self.configured = true;
				self.storage_warning =
					Some("Could not restore message-history preferences; capture remains off");
			}
		}
		if let Some(account) = state.user.as_ref().map(|user| user.id) {
			self.sync_preferences(state.generation, account, messaging);
		}
	}

	pub fn accept_settings_saved(
		&mut self,
		generation: u64,
		result: Result<(), StoreError>,
		messaging: &mut ui::MessagingUi,
	) {
		self.settings_saving = false;
		match result {
			Ok(()) => {
				self.configured = true;
				if let Some(account) = messaging.message_history().account() {
					self.sync_preferences(generation, account, messaging);
				}
				messaging.message_history_mut().set_status(
					generation,
					"History preferences saved.",
					None,
					true,
				);
			}
			Err(_) => {
				self.settings.enabled = false;
				messaging.message_history_mut().set_status(
					generation,
					"Could not save history preferences; capture has been stopped.",
					None,
					true,
				);
			}
		}
	}

	pub fn accept_page(
		&mut self,
		result: Result<ui::message_history::HistoryPage, StoreError>,
		messaging: &mut ui::MessagingUi,
	) {
		match result {
			Ok(page) => {
				messaging.accept_message_history_page(page);
			}
			Err(_) => {
				let generation = messaging.message_history().generation();
				messaging
					.message_history_mut()
					.page_failed(generation, "Could not read this local history page.");
			}
		}
	}

	pub fn accept_clear(
		&mut self,
		generation: u64,
		result: Result<usize, StoreError>,
		state: &State,
		messaging: &mut ui::MessagingUi,
	) {
		let cleared = result.is_ok();
		match result {
			Ok(count) => {
				messaging.message_history_mut().set_status(
					generation,
					format!("Cleared {count} retained messages."),
					None,
					true,
				);
				messaging
					.message_history_mut()
					.open_all(generation, state.user.as_ref().map(|user| user.id));
			}
			Err(_) => messaging.message_history_mut().set_status(
				generation,
				"Could not clear retained history; report this before relying on deletion.",
				None,
				true,
			),
		}
		if cleared && let Some(account) = messaging.message_history().account() {
			self.sync_preferences(generation, account, messaging);
		}
	}

	pub fn accept_mutations(
		&mut self,
		generation: u64,
		result: Result<usize, StoreError>,
		messaging: &mut ui::MessagingUi,
	) {
		if result.is_err() {
			messaging.message_history_mut().set_status(
				generation,
				"Some history observations could not be saved; retrying is not silent evidence of persistence.",
				None,
				false,
			);
		}
	}

	pub fn accept_export(
		&mut self,
		generation: u64,
		result: Result<ExportSummary, ExportError>,
		messaging: &mut ui::MessagingUi,
	) {
		self.export = None;
		let (text, complete) = match result {
			Ok(summary) => (
				format!(
					"Exported {} retained rows ({} bytes).",
					summary.rows, summary.bytes
				),
				true,
			),
			Err(error) => (format!("History export failed: {}", error.message()), true),
		};
		messaging
			.message_history_mut()
			.set_status(generation, text, None, complete);
	}

	pub fn end_session(&mut self, account: Option<Id>, forget: bool) {
		self.pending.clear();
		self.pending_bytes = 0;
		if let Some(export) = self.export.take() {
			export.cancelled.store(true, Ordering::Release);
		}
		if forget && let Some(account) = account {
			self.forget_account_media(account);
		}
	}

	pub fn forget_account_media(&mut self, account: Id) {
		if let Some(media) = &self.media {
			media.cancel_account(account.0);
		}
		self.media_tickets
			.retain(|_, ticket| ticket.account != account);
		if self.media_cleanup.len() >= 8 {
			self.storage_warning =
				Some("Attachment cleanup queue is full; retry account removal later");
			return;
		}
		if let Ok(root) = local_store::data_dir() {
			let manager = match &self.media {
				Some(manager) => Ok(manager.clone()),
				None => crate::message_history_media::MediaManager::new(
					crate::message_history_media::MediaConfig::new(root),
				)
				.map(Arc::new),
			};
			match manager {
				Ok(media) => {
					media.cancel_account(account.0);
					let (send, receive) = std::sync::mpsc::sync_channel(1);
					if std::thread::Builder::new()
						.name("serein-history-cleanup".into())
						.spawn(move || {
							let result = media.clear_account(account.0);
							let _ = send.send((account, result));
						})
						.is_ok()
					{
						self.media_cleanup.push_back(receive);
					}
				}
				Err(_) => {
					self.storage_warning = Some("Could not start account attachment cleanup");
				}
			}
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn ignore_filters_are_unique_nonzero_ids_with_hard_count_bounds() {
		assert_eq!(
			parse_filters("20,30\n40 50").unwrap(),
			vec![Id(20), Id(30), Id(40), Id(50)]
		);
		assert_eq!(join_filters(&parse_filters("20,30").unwrap()), "20,30");
		assert!(parse_filters("20,20").is_err());
		assert!(parse_filters("0").is_err());
		assert!(parse_filters("not-an-id").is_err());
		let bounded = (1..=512)
			.map(|id| id.to_string())
			.collect::<Vec<_>>()
			.join(",");
		assert_eq!(parse_filters(&bounded).unwrap().len(), 512);
		let overflow = (1..=513)
			.map(|id| id.to_string())
			.collect::<Vec<_>>()
			.join(",");
		assert_eq!(
			parse_filters(&overflow),
			Err("Each ignore list may contain at most 512 IDs")
		);
		assert_eq!(
			parse_filters(&"9".repeat(ui::message_history::MAX_PREFERENCE_LIST_CHARS + 1)),
			Err("Ignore list is too long")
		);
	}
}
