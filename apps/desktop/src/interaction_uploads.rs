//! Native file selection scoped to the current application modal, separate from the composer.
use crate::{Desktop, uploads::Uploads};
use client_core::{Command, auth::AuthState};
use model::Id;
#[derive(Default)]
pub(crate) struct Files {
	scope: Option<(u64, Id, Id)>,
	fields: Vec<(String, Uploads)>,
}
impl Desktop {
	pub(crate) fn poll_interaction_files(&mut self, ctx: &eframe::egui::Context) {
		let scope = self.state.interactions.modal.as_ref().and_then(|modal| {
			self.state
				.selected
				.map(|channel| (self.state.generation, modal.id, channel))
		});
		if self.interaction_files.scope != scope {
			self.interaction_files = Files {
				scope,
				..Default::default()
			};
		}
		let allowed = self.state.auth == AuthState::Authenticated && self.state.gateway_connected;
		if self
			.interaction_files
			.fields
			.iter()
			.any(|(_, files)| files.busy())
			&& let Some(pending) = &mut self.state.interactions.pending
		{
			pending.deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
		}
		let mut offset = 0;
		for (custom_id, uploads) in &mut self.interaction_files.fields {
			uploads.poll(self.state.generation, self.state.selected, allowed, ctx);
			let files = uploads.files();
			let count = files.len();
			self.messaging.set_interaction_files(
				custom_id,
				files
					.into_iter()
					.enumerate()
					.map(|(i, (name, _))| (offset + i, name))
					.collect(),
			);
			offset += count;
			if let Some(error) = uploads.take_notice() {
				self.state.interactions.error = Some(error);
			}
		}
	}
	pub(crate) fn choose_interaction_files(&mut self, ctx: &eframe::egui::Context) {
		let Some(custom_id) = self.messaging.interaction_file_request.take() else {
			return;
		};
		if self.state.demo || self.fixture_only || self.state.interactions.busy() {
			return;
		}
		let Some((generation, modal, channel)) = self.interaction_files.scope else {
			return;
		};
		if generation != self.state.generation
			|| self
				.state
				.interactions
				.modal
				.as_ref()
				.is_none_or(|m| m.id != modal)
		{
			return;
		}
		if !self.state.can_view(channel) {
			return;
		}
		let index = if let Some(index) = self
			.interaction_files
			.fields
			.iter()
			.position(|(id, _)| *id == custom_id)
		{
			index
		} else {
			if self.interaction_files.fields.len() == 5 {
				return;
			}
			self.interaction_files
				.fields
				.push((custom_id.clone(), Uploads::default()));
			self.interaction_files.fields.len() - 1
		};
		let uploads = &mut self.interaction_files.fields[index].1;
		if uploads.busy() {
			return;
		}
		uploads.remove();
		self.messaging.set_interaction_files(&custom_id, vec![]);
		if let Err(error) = uploads.start_choose(
			generation,
			channel,
			self.runtime.handle(),
			ctx,
			self.window.clone(),
		) {
			self.state.interactions.error = Some(error);
		}
	}
	pub(crate) fn interaction_upload(&mut self, command: Command) {
		let channel = self.state.selected;
		let mut sources = vec![];
		if let Some(channel) = channel {
			for (_, uploads) in &mut self.interaction_files.fields {
				if let Some(mut selected) = uploads.take_source(self.state.generation, channel) {
					sources.append(&mut selected);
				}
			}
		}
		let allowed = !self.fixture_only
			&& !self.state.demo
			&& self.state.gateway_connected
			&& self.state.auth == AuthState::Authenticated;
		if !allowed
			|| sources.is_empty()
			|| sources.len() > 10
			|| sources
				.iter()
				.map(discord_api::upload::Source::size)
				.sum::<u64>()
				> discord_api::upload::MAX_TOTAL_BYTES
			|| self.connection.is_none()
		{
			self.state.command_rejected(command);
			self.state.interactions.error =
				Some("Files were not sent; select up to 10 files totaling at most 500 MB");
			return;
		}
		let (progress, receive) =
			tokio::sync::watch::channel(discord_api::upload::Status::Preparing);
		let (cancel, _) = tokio::sync::watch::channel(false);
		if self
			.interaction_files
			.fields
			.first_mut()
			.is_none_or(|(_, uploads)| uploads.begin_upload(receive, cancel.clone()).is_err())
		{
			self.state.command_rejected(command);
			return;
		}
		let request = crate::uploads::UploadRequest {
			command,
			source: sources,
			progress,
			cancel,
		};
		if let Err(error) = self.connection.as_ref().unwrap().uploads.try_send(request) {
			self.state.command_rejected(error.into_inner().command);
		}
	}
}
pub(crate) fn has_files(components: &[model::Component]) -> bool {
	components.iter().any(|c| {
		c.kind == 19 && !c.values.is_empty()
			|| has_files(&c.components)
			|| c.component
				.as_deref()
				.is_some_and(|c| has_files(std::slice::from_ref(c)))
	})
}
