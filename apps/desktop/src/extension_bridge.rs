//! Maps the bounded extension worker to native UI requests; no plugin runs here.
use crate::extensions::{Event, ExtensionHost, InstallSource, InstalledExtension, Job, Starter};
use client_core::State;
use eframe::egui;
use extensions::{
	AppEventKind, Capability, CatalogEntry, ExtensionKind, Invocation, MessageEvent, Surface,
};
use std::{
	collections::{BTreeMap, BTreeSet, VecDeque},
	path::PathBuf,
	sync::{Arc, mpsc},
	time::{Duration, Instant},
};
use ui::{ExtensionContext, ExtensionEntry, ExtensionRequest};

struct Pending {
	catalog: bool,
	theme_save: bool,
	generation: u64,
	cleanup: bool,
	reconcile: bool,
	preview: Option<(String, String)>,
	invocation: Option<(String, Invocation, ExtensionContext)>,
}
impl Pending {
	fn reactive(&self) -> bool {
		self.invocation
			.as_ref()
			.is_some_and(|(_, input, _)| input.message_event.is_some() || input.app_event.is_some())
	}
}

const MAX_MESSAGE_EVENTS: usize = 32;
const MAX_MESSAGE_EVENT_BYTES: usize = 64 * 1024;
const MESSAGE_EVENT_INTERVAL: Duration = Duration::from_millis(100);

enum ReactiveEvent {
	Message(MessageEvent),
	App(AppEventKind),
}
impl ReactiveEvent {
	fn available(&self, state: &State) -> bool {
		match self {
			Self::Message(_) => crate::extension_events::available(state),
			Self::App(_) => crate::extension_app::available(state),
		}
	}
}
struct QueuedEvent {
	id: String,
	action: String,
	context: ExtensionContext,
	event: ReactiveEvent,
}
impl QueuedEvent {
	fn bytes(&self) -> usize {
		std::mem::size_of::<Self>()
			+ self.id.len()
			+ self.action.len()
			+ match &self.event {
				ReactiveEvent::App(_) => 0,
				ReactiveEvent::Message(event) => {
					event.channel_id.len()
						+ event.message_id.len()
						+ event.author_id.as_ref().map_or(0, String::len)
						+ event.content.as_ref().map_or(0, String::len)
				}
			}
	}
}
type PickedBackground = (Vec<u8>, Arc<egui::ColorImage>);

enum ThemePickerResult {
	Image(Result<Option<PickedBackground>, String>),
	Cover(Result<Option<PickedBackground>, String>),
	Export(Option<PathBuf>, Box<extensions::Package>),
}
#[derive(Default)]
pub struct Bridge {
	host: Option<ExtensionHost>,
	scope: Option<(u64, Option<String>)>,
	pending: BTreeMap<u64, Pending>,
	installed: Vec<InstalledExtension>,
	catalog_page: Option<ExtensionKind>,
	catalog_refresh_pending: bool,
	starters: BTreeMap<String, Starter>,
	disabled: BTreeSet<String>,
	catalog: BTreeMap<String, CatalogEntry>,
	imported: Option<InstallSource>,
	picker: Option<mpsc::Receiver<Option<PathBuf>>>,
	theme_picker: Option<mpsc::Receiver<ThemePickerResult>>,
	theme_preview: Option<(Box<extensions::Theme>, Option<Arc<egui::ColorImage>>)>,
	message_events: VecDeque<QueuedEvent>,
	message_event_at: Option<Instant>,
	message_events_dropped: bool,
	app_key: Option<crate::extension_app::ChangeKey>,
	app_context_changed: bool,
	data_changes: crate::extension_data_events::Changes,
	data_key: Option<crate::extension_data_events::DataKey>,
}
impl Bridge {
	pub fn data_changed(&mut self, changes: crate::extension_data_events::Changes) {
		self.data_changes.merge(changes);
	}
	/// Permission changes retire copied app data and proposals before any further delivery.
	pub fn access_changed(&mut self, messaging: &mut ui::MessagingUi) {
		self.app_context_changed = true;
		self.message_events.clear();
		let cancelling = self.pending.values().any(|pending| {
			pending
				.invocation
				.as_ref()
				.is_some_and(|(_, input, _)| input.app.is_some() || input.app_event.is_some())
		});
		if cancelling {
			if let Some(host) = &mut self.host {
				host.cancel();
			}
			self.pending.retain(|_, pending| pending.cleanup);
		}
		for entry in &self.installed {
			if crate::extension_app::uses_app(&entry.manifest.capabilities) {
				messaging.extensions.remove_runtime(&entry.manifest.id);
			}
		}
	}
	fn scope_current(&self, state: &State) -> bool {
		self.scope.as_ref().is_some_and(|(generation, account)| {
			*generation == state.generation
				&& state.user.as_ref().is_some_and(|user| {
					account.as_deref().and_then(|id| id.parse::<u64>().ok()) == Some(user.id.0)
				})
		})
	}
	fn app_events(&mut self, state: &State, messaging: &ui::MessagingUi) {
		if !self.scope_current(state)
			|| !crate::extension_app::available(state)
			|| !self.installed.iter().any(|entry| {
				entry.error.is_none()
					&& !self.disabled.contains(&entry.manifest.id)
					&& entry.manifest.capabilities.contains(&Capability::AppEvents)
			}) {
			self.app_key = None;
			self.data_changes = Default::default();
			self.data_key = None;
			return;
		}
		let data_key = crate::extension_data_events::DataKey::capture(state);
		if let Some(old) = &self.data_key {
			self.data_changes.merge(data_key.changed(old));
		}
		self.data_key = Some(data_key);
		let changes = std::mem::take(&mut self.data_changes);
		let key = crate::extension_app::ChangeKey::capture(state, messaging);
		let invalidated = std::mem::take(&mut self.app_context_changed);
		let event = self
			.app_key
			.as_ref()
			.map_or(Some(AppEventKind::Ready), |old| {
				key.changed(old)
					.or_else(|| invalidated.then_some(AppEventKind::Context))
			});
		self.app_key = Some(key);
		for entry in &self.installed {
			if entry.error.is_some()
				|| self.disabled.contains(&entry.manifest.id)
				|| !entry.manifest.capabilities.contains(&Capability::AppEvents)
			{
				continue;
			}
			let Some(action) = entry
				.manifest
				.actions
				.iter()
				.find(|a| a.surface == Surface::AppEvent)
			else {
				continue;
			};
			let detailed = entry
				.manifest
				.capabilities
				.contains(&Capability::DataEvents);
			let mut kinds = [None; 8];
			kinds[0] = event;
			for (index, kind) in changes.kinds(&entry.manifest.capabilities).enumerate() {
				if detailed {
					kinds[index + 1] = Some(kind);
				} else {
					kinds[0] = kinds[0].or(Some(AppEventKind::Context));
					break;
				}
			}
			for kind in kinds.into_iter().flatten() {
				// Coalesce invalidation hints, never copied data. Old observers retain one latest event.
				self.message_events.retain(|event| {
					event.id != entry.manifest.id
						|| !matches!(event.event, ReactiveEvent::App(old) if !detailed || old == kind)
				});
				let queued = QueuedEvent {
					id: entry.manifest.id.clone(),
					action: action.id.clone(),
					context: ExtensionContext::capture(state, false),
					event: ReactiveEvent::App(kind),
				};
				if self.message_events.len() >= MAX_MESSAGE_EVENTS
					|| self
						.message_events
						.iter()
						.map(QueuedEvent::bytes)
						.sum::<usize>() + queued.bytes()
						> MAX_MESSAGE_EVENT_BYTES
				{
					self.message_events_dropped = true;
				} else {
					self.message_events.push_back(queued);
				}
			}
		}
	}
	fn subscribes(&self, entry: &InstalledExtension) -> bool {
		entry.error.is_none()
			&& !self.disabled.contains(&entry.manifest.id)
			&& entry
				.manifest
				.capabilities
				.contains(&Capability::MessageEvents)
			&& entry
				.manifest
				.actions
				.iter()
				.any(|action| action.surface == Surface::MessageEvent)
	}
	pub fn has_message_events(&self, state: &State) -> bool {
		self.installed.iter().any(|entry| self.subscribes(entry))
			&& crate::extension_events::available(state)
			&& self.scope_current(state)
	}
	pub fn message_event(&mut self, state: &State, event: MessageEvent) {
		if !self.has_message_events(state) || event.validate().is_err() {
			return;
		}
		for entry in &self.installed {
			if !self.subscribes(entry) {
				continue;
			}
			let action = entry
				.manifest
				.actions
				.iter()
				.find(|action| action.surface == Surface::MessageEvent)
				.unwrap();
			let queued = QueuedEvent {
				id: entry.manifest.id.clone(),
				action: action.id.clone(),
				context: ExtensionContext::capture(state, false),
				event: ReactiveEvent::Message(event.clone()),
			};
			if self.message_events.len() >= MAX_MESSAGE_EVENTS
				|| self
					.message_events
					.iter()
					.map(QueuedEvent::bytes)
					.sum::<usize>() + queued.bytes()
					> MAX_MESSAGE_EVENT_BYTES
			{
				self.message_events_dropped = true;
				continue;
			}
			self.message_events.push_back(queued);
		}
	}
	pub fn cancel_stale_message_events(&mut self, state: &State) {
		let available = self.scope_current(state);
		self.message_events.retain(|event| {
			available && event.event.available(state) && event.context.is_current(state)
		});
		if self.pending.values().any(|pending| {
			pending.reactive()
				&& pending
					.invocation
					.as_ref()
					.is_some_and(|(_, input, context)| {
						!available
							|| !context.is_current(state)
							|| input.message_event.is_some()
								&& !crate::extension_events::available(state)
							|| input.app_event.is_some() && !crate::extension_app::available(state)
					})
		}) && let Some(host) = &mut self.host
		{
			host.cancel();
		}
	}
	pub fn cleanup_pending(&self) -> bool {
		self.pending.values().any(|pending| pending.cleanup)
	}
	pub fn logout(&mut self, ctx: &egui::Context) -> Result<(), String> {
		self.message_events.clear();
		self.app_key = None;
		self.app_context_changed = false;
		self.data_changes = Default::default();
		self.data_key = None;
		for entry in &mut self.installed {
			entry.preserve_deleted_messages = false;
			entry.image_sharing = false;
		}
		self.picker = None;
		self.theme_picker = None;
		self.theme_preview = None;
		self.pending.retain(|_, pending| pending.cleanup);
		if let Some(host) = &mut self.host {
			host.cancel();
			if let Some((generation, Some(account))) = &self.scope {
				let token = host.submit(
					Job::Logout {
						account: account.clone(),
					},
					ctx,
				)?;
				self.pending.insert(
					token,
					Pending {
						catalog: false,
						theme_save: false,
						generation: *generation,
						cleanup: true,
						reconcile: false,
						preview: None,
						invocation: None,
					},
				);
			}
		}
		Ok(())
	}
	pub fn tick(
		&mut self,
		state: &mut State,
		messaging: &mut ui::MessagingUi,
		ctx: &egui::Context,
		runtime: &tokio::runtime::Runtime,
		window: &Arc<winit::window::Window>,
		demo: bool,
	) {
		messaging.image_sharing_enabled = false;
		let account = state
			.user
			.as_ref()
			.filter(|_| state.demo || state.auth == client_core::auth::AuthState::Authenticated)
			.map(|u| u.id.0.to_string());
		if self.host.is_none() {
			let root = if demo {
				Some(std::env::temp_dir().join("serein-extension-demo"))
			} else {
				dirs::data_local_dir().map(|root| root.join("serein").join("extensions"))
			};
			let Some(root) = root else {
				messaging.extensions.status = "Application data directory is unavailable.".into();
				return;
			};
			self.host = Some(ExtensionHost::new(root));
		}
		let scope = (state.generation, account.clone());
		if self.scope.as_ref() != Some(&scope) {
			self.message_events.clear();
			self.app_key = None;
			self.app_context_changed = false;
			self.data_changes = Default::default();
			self.data_key = None;
			self.message_events_dropped = false;
			state.set_preserve_deleted_messages(false);
			self.cancel_previews(messaging);
			self.host.as_mut().unwrap().cancel();
			self.pending.retain(|_, pending| pending.cleanup);
			self.picker = None;
			self.theme_picker = None;
			self.theme_preview = None;
			self.imported = None;
			self.installed.clear();
			self.disabled.clear();
			messaging.extensions.reset_runtime();
			self.scope = Some(scope);
			self.submit(
				Job::Load {
					account: account.clone(),
				},
				None,
				state.generation,
				ctx,
				messaging,
			);
			self.entries(messaging);
		}
		if self.pending.values().any(|pending| {
			pending
				.invocation
				.as_ref()
				.is_some_and(|(_, input, context)| {
					!context.is_current(state)
						|| input.message_event.is_some()
							&& !crate::extension_events::available(state)
				})
		}) {
			self.cancel_previews(messaging);
			self.host.as_mut().unwrap().cancel();
			self.pending.retain(|_, pending| pending.cleanup);
			self.message_events.clear();
			self.submit(
				Job::Load {
					account: account.clone(),
				},
				None,
				state.generation,
				ctx,
				messaging,
			);
			messaging.extensions.status =
				"Result discarded because the conversation or draft changed.".into();
		}
		while let Some((token, outcome)) = self.host.as_mut().unwrap().poll() {
			let pending = self.pending.remove(&token);
			if pending
				.as_ref()
				.is_none_or(|p| p.generation != state.generation)
			{
				if let Err(error) = outcome {
					messaging.extensions.status = error;
				}
				continue;
			}
			if let Some((id, sha256)) = pending.as_ref().and_then(|p| p.preview.as_ref()) {
				if self
					.catalog
					.get(id)
					.and_then(|e| e.preview.as_ref())
					.is_some_and(|p| p.sha256 == *sha256)
				{
					let image = match outcome {
						Ok(Event::Preview {
							id: returned,
							image,
						}) if returned == *id => image,
						_ => None,
					};
					messaging.extensions.receive_preview(id.clone(), image);
				} else if !self
					.pending
					.values()
					.any(|p| p.preview.as_ref().is_some_and(|(next, _)| next == id))
					&& !messaging.extensions.requests.iter().any(
						|request| matches!(request, ExtensionRequest::Preview { id: next } if next == id),
					) {
					messaging.extensions.retry_preview(id);
				}
				continue;
			}
			match outcome {
				Err(error) => {
					if let Some((id, _, _)) = pending.as_ref().and_then(|p| p.invocation.as_ref()) {
						self.disabled.insert(id.clone());
						self.apply_theme(ctx);
						self.entries(messaging);
					}
					if pending.as_ref().is_some_and(|pending| pending.catalog) {
						messaging.extensions.status =
							format!("Catalog refresh failed; keeping saved packages. {error}");
					} else {
						messaging.extensions.report_error(error);
					}
					if pending.is_some_and(|p| p.reconcile) {
						self.submit(
							Job::Load {
								account: account.clone(),
							},
							None,
							state.generation,
							ctx,
							messaging,
						);
					}
				}
				Ok(Event::Loaded {
					catalog,
					installed,
					starters,
				}) => {
					if let Some(catalog) = catalog {
						self.catalog = catalog
							.entries
							.into_iter()
							.map(|entry| (entry.manifest.id.clone(), entry))
							.collect();
					}
					self.starters = starters
						.into_iter()
						.map(|entry| (source_id(&entry.source).to_owned(), entry))
						.collect();
					self.disabled = installed
						.iter()
						.filter(|e| e.error.is_some())
						.map(|e| e.manifest.id.clone())
						.collect();
					if let Some(error) = installed.iter().find_map(|e| e.error.as_ref()) {
						messaging.extensions.status = error.clone();
					}
					self.installed = installed;
					self.apply_theme(ctx);
					self.entries(messaging);
				}
				Ok(Event::Catalog(catalog)) => {
					self.catalog = catalog
						.entries
						.into_iter()
						.map(|entry| (entry.manifest.id.clone(), entry))
						.collect();
					self.entries(messaging);
					// A finished refresh is visible in the grid; only failures need a status line.
					messaging.extensions.status.clear();
				}
				Ok(Event::Imported {
					manifest,
					source,
					download_bytes,
				}) => {
					let sha256 = source_hash(&source).to_owned();
					self.imported = Some(source);
					messaging.extensions.offer_import(ExtensionEntry {
						cover_image: None,
						local_theme: false,
						manifest,
						description: String::new(),
						preview: None,
						theme_preview: None,
						sha256,
						reviewed: false,
						download_bytes,
						enabled: false,
						update_available: false,
						update_manifest: None,
						cleanup_pending: false,
					});
					messaging.extensions.status =
						"Package inspected. Review its source and capabilities before enabling."
							.into();
				}
				Ok(Event::ThemeSelected {
					id,
					background_image,
				}) => {
					self.theme_preview = None;
					for entry in &mut self.installed {
						entry.active_theme = id.as_deref() == Some(entry.manifest.id.as_str());
						entry.background_image = if entry.active_theme {
							background_image.clone()
						} else {
							None
						};
					}
					self.apply_theme(ctx);
					self.entries(messaging);
				}
				Ok(Event::Enabled(installed)) => {
					self.app_key = None;
					self.message_events
						.retain(|event| event.id != installed.manifest.id);
					if pending.as_ref().is_some_and(|p| p.theme_save) {
						self.theme_preview = None;
						messaging.extensions.theme_saved(&installed.manifest.id);
					}
					messaging.extensions.remove_runtime(&installed.manifest.id);
					let theme = installed.active_theme;
					if theme {
						for old in &mut self.installed {
							old.active_theme = false;
							old.background_image = None;
						}
					}
					self.installed
						.retain(|old| old.manifest.id != installed.manifest.id);
					self.disabled.remove(&installed.manifest.id);
					self.installed.push(installed);
					self.imported = None;
					self.apply_theme(ctx);
					self.entries(messaging);
					messaging.extensions.status = "Extension enabled.".into();
				}
				Ok(Event::Disabled(id)) => {
					let theme = self.installed.iter().any(|entry| {
						entry.manifest.id == id && entry.manifest.kind == ExtensionKind::Theme
					});
					self.installed.retain(|entry| entry.manifest.id != id);
					self.disabled.remove(&id);
					messaging.extensions.remove_runtime(&id);
					self.apply_theme(ctx);
					self.entries(messaging);
					// The gallery reports the outcome; an open theme editor is a different task.
					if !messaging.extensions.editing_theme() {
						messaging.extensions.status = if theme {
							"Theme removed.".into()
						} else {
							"Disabled. Downloaded code and extension data were removed.".into()
						};
					}
				}
				Ok(Event::Invoked { id, output }) => {
					if let Some((requested, invocation, context)) =
						pending.and_then(|p| p.invocation)
						&& requested == id && !self.disabled.contains(&id)
						&& self.installed.iter().any(|e| e.manifest.id == id)
					{
						if context.is_current(state)
							&& let Some(appearance) = &output.appearance
						{
							if let Some(installed) = self
								.installed
								.iter_mut()
								.find(|entry| entry.manifest.id == id)
							{
								installed.theme = Some(appearance.clone());
							}
							self.apply_theme(ctx);
						}
						if invocation.message_event.is_none() && invocation.app_event.is_none() {
							messaging
								.extensions
								.present_output(id, invocation, context, output, state);
						}
					}
				}
				Ok(Event::EditTheme {
					package,
					image,
					cover,
					local_theme,
					preview,
				}) => messaging.extensions.receive_theme_edit(
					package,
					image,
					cover,
					local_theme,
					preview,
				),
				Ok(Event::ThemeExported) => messaging.extensions.status = "Theme exported.".into(),
				Ok(Event::LoggedOut | Event::Preview { .. }) => {}
			}
		}
		if let Some(receiver) = &self.picker {
			match receiver.try_recv() {
				Ok(path) => {
					self.picker = None;
					if let Some(path) = path {
						self.submit(
							Job::InspectImport { path },
							None,
							state.generation,
							ctx,
							messaging,
						);
					}
				}
				Err(mpsc::TryRecvError::Disconnected) => {
					self.picker = None;
					messaging.extensions.status = "Extension file selection ended.".into();
				}
				Err(mpsc::TryRecvError::Empty) => {}
			}
		}
		if let Some(receiver) = &self.theme_picker {
			match receiver.try_recv() {
				Ok(result) => {
					self.theme_picker = None;
					match result {
						ThemePickerResult::Image(Ok(Some((bytes, image)))) => {
							messaging.extensions.receive_theme_image(bytes, image)
						}
						ThemePickerResult::Cover(Ok(Some((bytes, image)))) => {
							messaging.extensions.receive_theme_cover(bytes, image)
						}
						ThemePickerResult::Image(Err(error)) => {
							messaging.extensions.report_error(error)
						}
						ThemePickerResult::Cover(Err(error)) => {
							messaging.extensions.report_error(error)
						}
						ThemePickerResult::Export(Some(path), package) => self.submit(
							Job::ExportTheme { path, package },
							None,
							state.generation,
							ctx,
							messaging,
						),
						_ => {}
					}
				}
				Err(mpsc::TryRecvError::Disconnected) => {
					self.theme_picker = None;
					messaging
						.extensions
						.report_error("Theme file selection ended.".into());
				}
				Err(mpsc::TryRecvError::Empty) => {}
			}
		}
		for request in std::mem::take(&mut messaging.extensions.requests) {
			if !matches!(request, ExtensionRequest::Preview { .. })
				&& !self.pending.is_empty()
				&& self.pending.values().all(|pending| {
					pending.preview.is_some() || pending.catalog || pending.reactive()
				}) {
				self.cancel_previews(messaging);
				self.host.as_mut().unwrap().cancel();
				self.pending.clear();
			}
			match request {
				ExtensionRequest::PreviewTheme { theme, image } => {
					self.theme_preview = theme.map(|theme| (theme, image));
					self.apply_theme(ctx);
				}
				ExtensionRequest::EditTheme { id, preview } => self.submit(
					Job::EditTheme { id, preview },
					None,
					state.generation,
					ctx,
					messaging,
				),
				ExtensionRequest::SaveTheme { package } => self.submit(
					Job::SaveTheme { package },
					None,
					state.generation,
					ctx,
					messaging,
				),
				ExtensionRequest::PickThemeImage if self.theme_picker.is_none() => {
					let (send, receive) = mpsc::sync_channel(1);
					let future = platform::save::theme_background_source(window.clone());
					let ctx = ctx.clone();
					runtime.spawn(async move {
						let result = if let Some(path) = future.await {
							tokio::task::spawn_blocking(move || {
								crate::extensions::read_background(&path)
									.map(|(bytes, image)| Some((bytes, Arc::new(image))))
							})
							.await
							.unwrap_or_else(|_| Err("Background image worker failed.".into()))
						} else {
							Ok(None)
						};
						let _ = send.send(ThemePickerResult::Image(result));
						ctx.request_repaint();
					});
					self.theme_picker = Some(receive);
				}
				ExtensionRequest::PickThemeCover if self.theme_picker.is_none() => {
					let (send, receive) = mpsc::sync_channel(1);
					let future = platform::save::theme_cover_source(window.clone());
					let ctx = ctx.clone();
					runtime.spawn(async move {
						let result = if let Some(path) = future.await {
							tokio::task::spawn_blocking(move || {
								crate::extensions::read_cover(&path)
									.map(|(bytes, image)| Some((bytes, Arc::new(image))))
							})
							.await
							.unwrap_or_else(|_| Err("Cover image worker failed.".into()))
						} else {
							Ok(None)
						};
						let _ = send.send(ThemePickerResult::Cover(result));
						ctx.request_repaint();
					});
					self.theme_picker = Some(receive);
				}
				ExtensionRequest::ExportTheme { package } if self.theme_picker.is_none() => {
					let (send, receive) = mpsc::sync_channel(1);
					let future = platform::save::theme_destination(
						window.clone(),
						&format!("{}.serein-extension", package.manifest.id),
					);
					let ctx = ctx.clone();
					runtime.spawn(async move {
						let _ = send.send(ThemePickerResult::Export(future.await, package));
						ctx.request_repaint();
					});
					self.theme_picker = Some(receive);
				}
				ExtensionRequest::PickThemeImage
				| ExtensionRequest::PickThemeCover
				| ExtensionRequest::ExportTheme { .. } => {}
				ExtensionRequest::SelectTheme { id } => {
					self.submit(
						Job::SelectTheme { id },
						None,
						state.generation,
						ctx,
						messaging,
					);
				}
				ExtensionRequest::RefreshCatalog => {
					self.submit(
						Job::RefreshCatalog { demo },
						None,
						state.generation,
						ctx,
						messaging,
					);
				}
				ExtensionRequest::Preview { id } => {
					if self.picker.is_some()
						|| self
							.pending
							.values()
							.any(|pending| pending.preview.is_none())
					{
						messaging.extensions.retry_preview(&id);
						continue;
					}
					if let Some(preview) = self
						.catalog
						.get(&id)
						.and_then(|entry| entry.preview.clone())
					{
						self.submit(
							Job::Preview { id, preview, demo },
							None,
							state.generation,
							ctx,
							messaging,
						);
					} else {
						messaging.extensions.receive_preview(id, None);
					}
				}
				ExtensionRequest::Import if self.picker.is_none() => {
					let (send, receive) = mpsc::sync_channel(1);
					let future = platform::save::extension_source(window.clone());
					let ctx = ctx.clone();
					runtime.spawn(async move {
						let _ = send.send(future.await);
						ctx.request_repaint();
					});
					self.picker = Some(receive);
				}
				ExtensionRequest::Import => {}
				ExtensionRequest::Enable {
					id,
					grants,
					sha256,
					reviewed,
				} => {
					self.message_events.retain(|event| event.id != id);
					let source = self.source_for(&id, &sha256, reviewed);
					if let Some(source) = source {
						if demo && matches!(source, InstallSource::Catalog(_)) {
							messaging.extensions.status =
								"Offline demo: import the local package instead.".into();
							continue;
						}
						self.submit(
							Job::Enable {
								source: Box::new(source),
								grants,
								account: account.clone(),
							},
							None,
							state.generation,
							ctx,
							messaging,
						);
					} else {
						messaging.extensions.status =
							"Refresh the catalog or import this package again.".into();
					}
				}
				ExtensionRequest::Disable { id } => {
					self.message_events.retain(|event| event.id != id);
					if let Some(entry) = self.installed.iter().find(|e| e.manifest.id == id) {
						let kind = entry.manifest.kind;
						self.cancel_previews(messaging);
						self.pending.retain(|_, pending| pending.cleanup);
						messaging.extensions.remove_runtime(&id);
						self.disabled.insert(id.clone());
						self.apply_theme(ctx);
						self.entries(messaging);
						self.submit(
							Job::Disable {
								id,
								kind,
								account: account.clone(),
							},
							None,
							state.generation,
							ctx,
							messaging,
						);
					}
				}
				ExtensionRequest::Invoke {
					id,
					mut invocation,
					mut context,
				} => {
					if !context.is_current(state)
						|| self.disabled.contains(&id)
						|| !self.installed.iter().any(|e| e.manifest.id == id)
					{
						continue;
					}
					if let Some(account) = &account {
						let manifest = &self
							.installed
							.iter()
							.find(|e| e.manifest.id == id)
							.unwrap()
							.manifest;
						invocation.app = crate::extension_app::snapshot(state, messaging, manifest);
						if crate::extension_app::uses_app(&manifest.capabilities) {
							// App snapshots and proposals belong to the conversation that produced them.
							context.app_wide = false;
							context.channel = state.selected;
						}
						let pending = Some((id.clone(), invocation.clone(), context));
						self.submit(
							Job::Invoke {
								id,
								account: account.clone(),
								invocation,
							},
							pending,
							state.generation,
							ctx,
							messaging,
						);
					}
				}
			}
		}
		messaging.image_sharing_enabled = account.is_some()
			&& self.installed.iter().any(|entry| {
				entry.error.is_none()
					&& !self.disabled.contains(&entry.manifest.id)
					&& entry.image_sharing
			});
		state.set_preserve_deleted_messages(
			account.is_some()
				&& self.installed.iter().any(|entry| {
					entry.error.is_none()
						&& !self.disabled.contains(&entry.manifest.id)
						&& entry.preserve_deleted_messages
				}),
		);
		let max_texture = ctx.input(|input| input.max_texture_side);
		if self
			.installed
			.iter()
			.filter_map(|entry| entry.background_image.as_ref())
			.any(|image| image.size.iter().any(|side| *side > max_texture))
		{
			messaging.extensions.status = format!(
				"This device supports background images up to {max_texture} pixels per edge. Choose a smaller image."
			);
		}
		messaging.extensions.busy = self.picker.is_some()
			|| self.theme_picker.is_some()
			|| self.pending.values().any(|pending| {
				pending.preview.is_none() && !pending.catalog && !pending.reactive()
			});
		messaging.extensions.catalog_refreshing =
			self.pending.values().any(|pending| pending.catalog);
		if self.refresh_catalog_on_open(
			messaging.extension_settings_page(),
			demo,
			messaging.extensions.busy || messaging.extensions.catalog_refreshing,
		) {
			self.submit(
				Job::RefreshCatalog { demo },
				None,
				state.generation,
				ctx,
				messaging,
			);
			messaging.extensions.catalog_refreshing = true;
		}
		if !self.host.as_ref().unwrap().busy() {
			self.pending.retain(|_, pending| pending.cleanup);
		}
		self.app_events(state, messaging);
		let available = self.scope_current(state);
		let installed = &self.installed;
		let disabled = &self.disabled;
		self.message_events.retain(|event| {
			available
				&& event.event.available(state)
				&& event.context.is_current(state)
				&& !disabled.contains(&event.id)
				&& installed
					.iter()
					.any(|entry| entry.manifest.id == event.id && entry.error.is_none())
		});
		if std::mem::take(&mut self.message_events_dropped) {
			messaging.extensions.status =
				"Some plugin events were skipped because the event queue was full.".into();
		}
		if !self.message_events.is_empty() {
			let wait = self.message_event_at.map_or(Duration::ZERO, |at| {
				MESSAGE_EVENT_INTERVAL.saturating_sub(at.elapsed())
			});
			if wait.is_zero() && !self.host.as_ref().unwrap().busy() {
				let queued = self.message_events.pop_front().unwrap();
				if let Some(account) = account {
					let mut invocation = Invocation {
						action: queued.action,
						..Default::default()
					};
					match queued.event {
						ReactiveEvent::Message(event) => {
							invocation.message_event = Some(Box::new(event))
						}
						ReactiveEvent::App(kind) => {
							invocation.app_event = Some(kind);
							let manifest = &self
								.installed
								.iter()
								.find(|e| e.manifest.id == queued.id)
								.unwrap()
								.manifest;
							invocation.app =
								crate::extension_app::snapshot(state, messaging, manifest);
						}
					}
					let pending = Some((queued.id.clone(), invocation.clone(), queued.context));
					self.submit(
						Job::Invoke {
							id: queued.id,
							account,
							invocation,
						},
						pending,
						state.generation,
						ctx,
						messaging,
					);
					self.message_event_at = Some(Instant::now());
				}
			} else if !wait.is_zero() {
				ctx.request_repaint_after(wait);
			}
		}
	}
	fn refresh_catalog_on_open(
		&mut self,
		page: Option<ExtensionKind>,
		demo: bool,
		busy: bool,
	) -> bool {
		if page != self.catalog_page {
			self.catalog_refresh_pending = page.is_some();
		}
		self.catalog_page = page;
		if demo {
			self.catalog_refresh_pending = false;
		}
		if self.catalog_refresh_pending && !busy {
			self.catalog_refresh_pending = false;
			return true;
		}
		false
	}

	fn cancel_previews(&self, messaging: &mut ui::MessagingUi) {
		for (id, _) in self
			.pending
			.values()
			.filter_map(|pending| pending.preview.as_ref())
		{
			messaging.extensions.retry_preview(id);
		}
	}
	fn submit(
		&mut self,
		job: Job,
		invocation: Option<(String, Invocation, ExtensionContext)>,
		generation: u64,
		ctx: &egui::Context,
		messaging: &mut ui::MessagingUi,
	) {
		let preview = match &job {
			Job::Preview { id, preview, .. } => Some((id.clone(), preview.sha256.clone())),
			_ => None,
		};
		let cleanup = matches!(job, Job::Disable { .. } | Job::Logout { .. });
		let theme_save = matches!(job, Job::SaveTheme { .. });
		let catalog = matches!(job, Job::RefreshCatalog { .. });
		let reconcile =
			cleanup || theme_save || matches!(job, Job::Enable { .. } | Job::SelectTheme { .. });
		match self.host.as_mut().unwrap().submit(job, ctx) {
			Ok(token) => {
				self.pending.insert(
					token,
					Pending {
						catalog,
						theme_save,
						cleanup,
						reconcile,
						preview,
						generation,
						invocation,
					},
				);
			}
			Err(error) => {
				if let Some((id, _)) = preview {
					messaging.extensions.receive_preview(id, None);
				} else {
					messaging.extensions.report_error(error);
				}
			}
		}
	}
	fn source_for(&self, id: &str, sha256: &str, reviewed: bool) -> Option<InstallSource> {
		self.starters
			.get(id)
			.filter(|entry| reviewed && source_hash(&entry.source) == sha256)
			.map(|entry| entry.source.clone())
			.or_else(|| {
				self.imported
					.as_ref()
					.filter(|source| {
						!reviewed && source_id(source) == id && source_hash(source) == sha256
					})
					.cloned()
			})
			.or_else(|| {
				self.catalog
					.get(id)
					.filter(|entry| reviewed && entry.sha256 == sha256)
					.cloned()
					.map(InstallSource::Catalog)
			})
	}

	fn entries(&self, messaging: &mut ui::MessagingUi) {
		messaging.extensions.active_theme = self
			.installed
			.iter()
			.find(|entry| entry.active_theme && !self.disabled.contains(&entry.manifest.id))
			.map(|entry| entry.manifest.id.clone());
		let mut entries: BTreeMap<String, ExtensionEntry> = self
			.catalog
			.iter()
			.map(|(id, entry)| {
				(
					id.clone(),
					ExtensionEntry {
						manifest: entry.manifest.clone(),
						description: entry.description.clone(),
						preview: entry.preview.clone(),
						theme_preview: None,
						cover_image: None,
						local_theme: false,
						sha256: entry.sha256.clone(),
						reviewed: true,
						download_bytes: entry.download_bytes,
						enabled: false,
						update_available: false,
						update_manifest: None,
						cleanup_pending: false,
					},
				)
			})
			.collect();
		for (id, starter) in &self.starters {
			if entries.contains_key(id) {
				continue;
			}
			let InstallSource::Bundled {
				manifest, sha256, ..
			} = &starter.source
			else {
				continue;
			};
			entries.insert(
				id.clone(),
				ExtensionEntry {
					manifest: manifest.clone(),
					description: starter.description.into(),
					preview: None,
					theme_preview: starter.theme.clone(),
					cover_image: None,
					local_theme: false,
					sha256: sha256.clone(),
					reviewed: true,
					download_bytes: starter.download_bytes,
					enabled: false,
					update_available: false,
					update_manifest: None,
					cleanup_pending: false,
				},
			);
		}
		for installed in &self.installed {
			let available = entries.get(&installed.manifest.id).filter(|entry| {
				!installed.local_theme
					&& installed.reviewed
					&& entry.manifest.kind == installed.manifest.kind
			});
			let update = available.filter(|entry| {
				entry.manifest.version != installed.manifest.version
					|| !entry.sha256.eq_ignore_ascii_case(&installed.sha256)
			});
			let entry = ExtensionEntry {
				manifest: installed.manifest.clone(),
				description: available.map_or_else(String::new, |entry| entry.description.clone()),
				preview: available.and_then(|entry| entry.preview.clone()),
				theme_preview: installed.theme.clone(),
				cover_image: installed.cover_image.clone(),
				local_theme: installed.local_theme,
				sha256: available
					.map_or_else(|| installed.sha256.clone(), |entry| entry.sha256.clone()),
				reviewed: available.map_or(installed.reviewed, |entry| entry.reviewed),
				download_bytes: available
					.map_or(installed.download_bytes, |entry| entry.download_bytes),
				enabled: !self.disabled.contains(&installed.manifest.id),
				cleanup_pending: self.disabled.contains(&installed.manifest.id),
				update_available: update.is_some(),
				update_manifest: update.map(|entry| entry.manifest.clone()),
			};
			entries.insert(installed.manifest.id.clone(), entry);
		}
		messaging
			.extensions
			.set_entries(entries.into_values().collect());
	}
	fn apply_theme(&self, ctx: &egui::Context) {
		// Explicit theme first, then enabled plugin appearances in stable ID order.
		let mut entries: Vec<_> = self
			.installed
			.iter()
			.filter(|entry| {
				entry.error.is_none()
					&& !self.disabled.contains(&entry.manifest.id)
					&& entry.theme.is_some()
					&& (entry.manifest.kind == ExtensionKind::Plugin
						|| entry.active_theme && self.theme_preview.is_none())
			})
			.collect();
		entries.sort_by_key(|entry| {
			(
				entry.manifest.kind == ExtensionKind::Plugin,
				&entry.manifest.id,
			)
		});
		let mut appearance = self
			.theme_preview
			.as_ref()
			.map_or_else(extensions::Theme::default, |(theme, _)| (**theme).clone());
		for entry in &entries {
			appearance.overlay(entry.theme.as_ref().unwrap());
		}
		ui::design::set_extension_theme(
			(!entries.is_empty() || self.theme_preview.is_some()).then_some(&appearance),
		);
		ui::design::set_background_image(
			ctx,
			self.theme_preview.as_ref().map_or_else(
				|| {
					entries
						.iter()
						.find(|entry| entry.active_theme)
						.and_then(|entry| entry.background_image.clone())
				},
				|(_, image)| image.clone(),
			),
		);
		ui::design::apply(ctx);
	}
}
fn source_id(source: &InstallSource) -> &str {
	match source {
		InstallSource::Catalog(entry) => &entry.manifest.id,
		InstallSource::Local { manifest, .. } | InstallSource::Bundled { manifest, .. } => {
			&manifest.id
		}
	}
}

fn source_hash(source: &InstallSource) -> &str {
	match source {
		InstallSource::Catalog(entry) => &entry.sha256,
		InstallSource::Local { sha256, .. } | InstallSource::Bundled { sha256, .. } => sha256,
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	fn message_events_fixture() -> (Bridge, State, MessageEvent) {
		let state = test_support::demo_state();
		let manifest = serde_json::from_str(include_str!(
			"../../../examples/extensions/message-counter/manifest.json"
		))
		.unwrap();
		let installed = InstalledExtension {
			background_image: None,
			cover_image: None,
			local_theme: false,
			active_theme: false,
			manifest,
			theme: None,
			reviewed: false,
			sha256: "a".repeat(64),
			download_bytes: 1,
			error: None,
			preserve_deleted_messages: false,
			image_sharing: false,
		};
		let bridge = Bridge {
			scope: Some((
				state.generation,
				Some(state.user.as_ref().unwrap().id.0.to_string()),
			)),
			installed: vec![installed],
			..Default::default()
		};
		let event = MessageEvent {
			kind: extensions::MessageEventKind::Create,
			channel_id: state.selected.unwrap().0.to_string(),
			message_id: "2".into(),
			author_id: Some("3".into()),
			content: Some("Synthetic message".into()),
		};
		assert!(bridge.has_message_events(&state));
		(bridge, state, event)
	}

	#[test]
	fn extension_app_events_coalesce_and_share_message_queue_limits() {
		let (mut bridge, mut state, event) = message_events_fixture();
		let mut messaging = ui::MessagingUi::default();
		bridge.installed[0]
			.manifest
			.capabilities
			.push(Capability::AppEvents);
		bridge.installed[0]
			.manifest
			.actions
			.push(extensions::Action {
				id: "app-event".into(),
				label: "Observe".into(),
				surface: Surface::AppEvent,
			});
		bridge.app_events(&state, &messaging);
		assert_eq!(bridge.message_events.len(), 1);
		assert!(matches!(
			bridge.message_events[0].event,
			ReactiveEvent::App(AppEventKind::Ready)
		));
		bridge.app_events(&state, &messaging);
		assert_eq!(bridge.message_events.len(), 1);
		messaging.reading_preferences.zoom_percent = 110;
		bridge.app_events(&state, &messaging);
		assert_eq!(bridge.message_events.len(), 1);
		assert!(matches!(
			bridge.message_events[0].event,
			ReactiveEvent::App(AppEventKind::Settings)
		));
		bridge.access_changed(&mut messaging);
		state.gateway_connected = false;
		bridge.app_events(&state, &messaging);
		assert!(matches!(
			bridge.message_events[0].event,
			ReactiveEvent::App(AppEventKind::Connection)
		));
		state.gateway_connected = true;
		bridge.app_events(&state, &messaging);
		for _ in 0..MAX_MESSAGE_EVENTS {
			bridge.message_event(&state, event.clone());
		}
		assert_eq!(bridge.message_events.len(), MAX_MESSAGE_EVENTS);
		assert!(bridge.message_events_dropped);
		bridge
			.disabled
			.insert(bridge.installed[0].manifest.id.clone());
		bridge.app_events(&state, &messaging);
		assert!(bridge.app_key.is_none());
		bridge.access_changed(&mut messaging);
		assert!(bridge.message_events.is_empty());
	}

	#[test]
	fn data_events_are_opt_in_granted_and_coalesce_per_kind() {
		let (mut bridge, state, _) = message_events_fixture();
		let messaging = ui::MessagingUi::default();
		let manifest = &mut bridge.installed[0].manifest;
		manifest.capabilities.extend([
			Capability::AppEvents,
			Capability::Members,
			Capability::Presence,
		]);
		manifest.actions.push(extensions::Action {
			id: "data-event".into(),
			label: "Observe".into(),
			surface: Surface::AppEvent,
		});
		bridge.app_events(&state, &messaging);
		bridge.message_events.clear();
		let envelope = client_core::Envelope {
			generation: state.generation,
			event: client_core::Event::Members(model::MemberList {
				guild: Some(model::Id(10)),
				channel: state.selected.unwrap(),
				request: state.member_request,
				slots: Vec::new(),
				start: 0,
				lazy: false,
				groups: Vec::new(),
				ranges: Vec::new(),
				total: 0,
				freshness: model::Freshness::Fresh,
			}),
		};
		let changes = crate::extension_data_events::Changes::capture(&state, &envelope);
		bridge.data_changed(changes);
		bridge.app_events(&state, &messaging);
		assert_eq!(bridge.message_events.len(), 1);
		assert!(matches!(
			bridge.message_events[0].event,
			ReactiveEvent::App(AppEventKind::Context)
		));
		bridge.message_events.clear();
		bridge.installed[0]
			.manifest
			.capabilities
			.push(Capability::DataEvents);
		for _ in 0..3 {
			bridge.data_changed(changes);
			bridge.app_events(&state, &messaging);
		}
		assert_eq!(bridge.message_events.len(), 2);
		assert!(matches!(
			bridge.message_events[0].event,
			ReactiveEvent::App(AppEventKind::Members)
		));
		assert!(matches!(
			bridge.message_events[1].event,
			ReactiveEvent::App(AppEventKind::Presence)
		));
		bridge.message_events.clear();
		bridge.installed[0]
			.manifest
			.capabilities
			.retain(|cap| *cap != Capability::Members);
		bridge.data_changed(changes);
		bridge.app_events(&state, &messaging);
		assert_eq!(bridge.message_events.len(), 1);
		assert!(matches!(
			bridge.message_events[0].event,
			ReactiveEvent::App(AppEventKind::Presence)
		));
	}

	#[test]
	fn selected_read_changes_are_observed_without_an_envelope() {
		let (mut bridge, mut state, _) = message_events_fixture();
		let messaging = ui::MessagingUi::default();
		let manifest = &mut bridge.installed[0].manifest;
		manifest.capabilities.extend([
			Capability::AppEvents,
			Capability::DataEvents,
			Capability::ReadState,
		]);
		manifest.actions.push(extensions::Action {
			id: "data-event".into(),
			label: "Observe".into(),
			surface: Surface::AppEvent,
		});
		let channel = state.selected.unwrap();
		state
			.apply_read_state(client_core::read_state::Event::Snapshot {
				entries: Some(vec![(channel, None, 3)]),
				version: None,
				partial: false,
			})
			.unwrap();
		bridge.app_events(&state, &messaging);
		bridge.message_events.clear();
		state
			.apply_read_state(client_core::read_state::Event::Ack {
				channel,
				message: Some(model::Id(99999)),
				manual: false,
				mention_count: Some(0),
				version: None,
			})
			.unwrap();
		bridge.app_events(&state, &messaging);
		assert_eq!(bridge.message_events.len(), 1);
		assert!(matches!(
			bridge.message_events[0].event,
			ReactiveEvent::App(AppEventKind::ReadState)
		));
		bridge.message_events.clear();
		bridge.app_events(&state, &messaging);
		assert!(bridge.message_events.is_empty());
	}

	#[test]
	fn message_event_queue_is_bounded_by_items_and_bytes() {
		let (mut bridge, state, mut event) = message_events_fixture();
		for _ in 0..MAX_MESSAGE_EVENTS + 1 {
			bridge.message_event(&state, event.clone());
		}
		assert_eq!(bridge.message_events.len(), MAX_MESSAGE_EVENTS);
		assert!(bridge.message_events_dropped);
		bridge.message_events.clear();
		bridge.message_events_dropped = false;
		event.content = Some("x".repeat(extensions::MAX_EVENT_CONTENT_BYTES));
		for _ in 0..4 {
			bridge.message_event(&state, event.clone());
		}
		assert_eq!(bridge.message_events.len(), 3);
		assert!(bridge.message_events_dropped);
		assert!(
			bridge
				.message_events
				.iter()
				.map(QueuedEvent::bytes)
				.sum::<usize>()
				<= MAX_MESSAGE_EVENT_BYTES
		);
		assert!(
			matches!(&bridge.message_events.front().unwrap().event, ReactiveEvent::Message(value) if *value == event)
		);
	}

	#[test]
	fn message_events_require_an_enabled_subscriber_in_the_current_scope() {
		for refusal in 0..7 {
			let (mut bridge, mut state, event) = message_events_fixture();
			match refusal {
				0 => bridge.installed.clear(),
				1 => bridge.installed[0].manifest.capabilities.clear(),
				2 => bridge.installed[0].manifest.actions.clear(),
				3 => bridge.installed[0].error = Some("Invalid package".into()),
				4 => {
					bridge.disabled.insert("message-counter".into());
				}
				5 => state.generation += 1,
				_ => bridge.scope.as_mut().unwrap().1 = Some("another-account".into()),
			}
			assert!(!bridge.has_message_events(&state));
			bridge.message_event(&state, event);
			assert!(bridge.message_events.is_empty());
		}
	}

	#[test]
	fn message_events_clear_when_conversation_session_or_access_changes() {
		for change in 0..4 {
			let (mut bridge, mut state, event) = message_events_fixture();
			bridge.message_event(&state, event);
			assert_eq!(bridge.message_events.len(), 1);
			match change {
				0 => state.selected = None,
				1 => state.generation += 1,
				2 => state.gateway_connected = false,
				_ => state.freshness = model::Freshness::Unavailable,
			}
			bridge.cancel_stale_message_events(&state);
			assert!(bridge.message_events.is_empty());
		}
		let (mut bridge, state, event) = message_events_fixture();
		bridge.message_event(&state, event);
		bridge.logout(&egui::Context::default()).unwrap();
		assert!(bridge.message_events.is_empty());
	}

	#[test]
	fn catalog_refresh_is_once_per_open_delayed_when_busy_and_offline_in_demo() {
		let mut bridge = Bridge::default();
		for page in [ExtensionKind::Theme, ExtensionKind::Plugin] {
			assert!(!bridge.refresh_catalog_on_open(Some(page), false, true));
			assert!(bridge.refresh_catalog_on_open(Some(page), false, false));
			assert!(!bridge.refresh_catalog_on_open(Some(page), false, false));
			assert!(!bridge.refresh_catalog_on_open(None, false, false));
			assert!(!bridge.refresh_catalog_on_open(Some(page), true, false));
			assert!(!bridge.refresh_catalog_on_open(None, false, false));
		}
	}

	#[test]
	fn cancelled_import_cannot_replace_the_catalog_bytes_the_user_approved() {
		let package =
			extensions::parse_package(include_bytes!("../../../extensions/ocean.serein-extension"))
				.unwrap();
		let manifest = package.manifest;
		let id = manifest.id.clone();
		let imported = InstallSource::Local {
			path: PathBuf::from("original.serein-extension"),
			sha256: "a".repeat(64),
			manifest: manifest.clone(),
		};
		let catalog = CatalogEntry {
			description: String::new(),
			preview: None,
			manifest,
			sha256: "b".repeat(64),
			source_commit: "c".repeat(40),
			download_bytes: 100,
			release_url: "https://example.org/release.json".into(),
		};
		let bridge = Bridge {
			imported: Some(imported),
			catalog: BTreeMap::from([(id.clone(), catalog)]),
			..Default::default()
		};
		assert!(matches!(
			bridge.source_for(&id, &"b".repeat(64), true),
			Some(InstallSource::Catalog(_))
		));
		assert!(matches!(
			bridge.source_for(&id, &"a".repeat(64), false),
			Some(InstallSource::Local { .. })
		));
		assert!(bridge.source_for(&id, &"a".repeat(64), true).is_none());
		assert!(bridge.source_for(&id, &"b".repeat(64), false).is_none());
		assert!(bridge.source_for(&id, &"d".repeat(64), true).is_none());
	}
}
