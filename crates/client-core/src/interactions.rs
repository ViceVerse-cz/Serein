//! One explicit interaction in flight. Modal drafts and private replies are session-only.
use crate::{
	Command, State,
	auth::{AuthState, Failure},
};
use model::{Component, Freshness, Id, Message};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

pub struct Request {
	pub request: u64,
	pub nonce: String,
	pub application_id: Id,
	pub channel_id: Id,
	pub guild_id: Option<Id>,
	pub message_id: Option<Id>,
	pub message_flags: u64,
	pub data: Data,
}
impl Request {
	pub fn valid(&self) -> bool {
		self.application_id.0 != 0
			&& self.channel_id.0 != 0
			&& self.guild_id.is_none_or(|id| id.0 != 0)
			&& self.nonce.len() <= 32
			&& !self.nonce.is_empty()
			&& self.nonce.bytes().all(|b| b.is_ascii_digit())
			&& match &self.data {
				Data::ApplicationCommand { invocation } => {
					self.message_id.is_none()
						&& self.message_flags == 0
						&& self.application_id == invocation.command.application_id
						&& invocation
							.command
							.guild_id
							.is_none_or(|guild| Some(guild) == self.guild_id)
						&& invocation.valid()
				}
				Data::Component {
					custom_id,
					component_type,
					values,
				} => {
					self.message_id.is_some_and(|id| id.0 != 0)
						&& !custom_id.is_empty()
						&& custom_id.chars().count() <= 100
						&& matches!(component_type, 2 | 3 | 5..=8)
						&& values.len() <= 25
						&& values.iter().all(|v| v.len() <= 400)
						&& (*component_type != 2 || values.is_empty())
				}
				Data::Modal {
					id,
					custom_id,
					components,
				} => {
					id.0 != 0
						&& !custom_id.is_empty()
						&& custom_id.chars().count() <= 100
						&& components.len() <= 5
						&& model::valid_components(components)
				}
			}
	}
}
pub enum Data {
	ApplicationCommand {
		invocation: Box<model::application_commands::Invocation>,
	},
	Component {
		custom_id: String,
		component_type: u8,
		values: Vec<String>,
	},
	Modal {
		id: Id,
		custom_id: String,
		components: Vec<Component>,
	},
}
#[derive(Clone)]
pub struct Modal {
	pub id: Id,
	pub application_id: Id,
	pub custom_id: String,
	pub title: String,
	pub components: Vec<Component>,
}
pub enum Event {
	Session(zeroize::Zeroizing<String>),
	Submitted {
		nonce: String,
		result: Result<(), Failure>,
	},
	Success {
		nonce: String,
	},
	Failed {
		nonce: String,
	},
	Modal {
		nonce: String,
		modal: Box<Modal>,
	},
	Ephemeral(Box<Message>),
}
impl Event {
	pub fn bytes(&self) -> usize {
		match self {
			Self::Session(s) => s.capacity(),
			Self::Submitted { nonce, .. } | Self::Success { nonce } | Self::Failed { nonce } => {
				nonce.capacity()
			}
			Self::Modal { nonce, modal } => {
				nonce.capacity()
					+ modal.title.capacity()
					+ modal.custom_id.capacity()
					+ model::component_bytes(&modal.components)
			}
			Self::Ephemeral(m) => m.bytes(),
		}
	}
}
pub struct Pending {
	pub nonce: String,
	pub channel: Id,
	pub application: Id,
	pub message: Option<Id>,
	pub deadline: Instant,
	modal: bool,
}
#[derive(Default)]
pub struct Interactions {
	pub pending: Option<Pending>,
	pub modal: Option<Modal>,
	pub error: Option<&'static str>,
	pub ephemeral: Vec<Message>,
	sequence: u64,
	modal_channel: Option<Id>,
	// A success acknowledgement can precede the modal dispatch.
	completed: Option<Pending>,
}
impl Interactions {
	pub fn busy(&self) -> bool {
		self.pending.is_some()
	}
	pub fn reset(&mut self) {
		self.pending = None;
		self.completed = None;
		self.modal = None;
		self.modal_channel = None;
		self.ephemeral.clear();
		self.error = None;
	}
}
fn valid_values(c: &Component, values: &[String], required: bool) -> bool {
	let min = c.min_values.unwrap_or(if required { 1 } else { 0 }) as usize;
	let max = c.max_values.unwrap_or(if c.kind == 22 {
		c.options.len() as u16
	} else {
		1
	}) as usize;
	(!required || !values.is_empty())
		&& (values.is_empty() && !required || values.len() >= min)
		&& values.len() <= max.min(25)
		&& values.iter().enumerate().all(|(i, v)| {
			!v.is_empty()
				&& v.len() <= 400
				&& !values[..i].contains(v)
				&& match c.kind {
					3 | 21 | 22 => c.options.iter().any(|o| o.value == *v),
					5..=8 => v.parse::<Id>().is_ok(),
					19 => v.parse::<usize>().is_ok_and(|n| n < 10),
					_ => false,
				}
		})
}
/// Validate edited inputs against the exact received form; never accept replacement schema.
pub fn valid_modal_values(source: &[Component], values: &[Component]) -> bool {
	source.len() == values.len()
		&& source.iter().zip(values).all(|(a, b)| {
			let mut schema = b.clone();
			schema.value = a.value.clone();
			schema.values = a.values.clone();
			schema.checked = a.checked;
			if matches!(a.kind, 1 | 18) {
				schema.components = a.components.clone();
				schema.component = a.component.clone();
			}
			if schema != *a {
				return false;
			}
			match a.kind {
				1 | 18 => {
					valid_modal_values(&a.components, &b.components)
						&& match (&a.component, &b.component) {
							(None, None) => true,
							(Some(a), Some(b)) => {
								valid_modal_values(std::slice::from_ref(a), std::slice::from_ref(b))
							}
							_ => false,
						}
				}
				10 => a == b,
				4 => {
					let text = b.value.as_deref().unwrap_or("");
					let len = text.chars().count();
					(!a.required || !text.is_empty())
						&& text.len() <= 16000
						&& len <= a.max_length.unwrap_or(4000).min(4000) as usize
						&& (len == 0 && !a.required
							|| len >= a.min_length.unwrap_or(u16::from(a.required)) as usize)
						&& b.values.is_empty()
						&& b.checked.is_none()
				}
				3 | 5..=8 | 19 | 22 => valid_values(a, &b.values, a.required),
				21 => b.value.as_ref().map_or(!a.required, |v| {
					valid_values(a, std::slice::from_ref(v), a.required)
				}),
				23 => b.checked.is_some(),
				_ => false,
			}
		})
}
impl State {
	pub(crate) fn interactions_allowed(&self) -> bool {
		self.auth == AuthState::Authenticated
			&& self.gateway_connected
			&& self.freshness == Freshness::Fresh
			&& self.selected.is_some_and(|id| self.can_view(id))
	}
	pub fn prepare_component(
		&mut self,
		message: Id,
		custom_id: &str,
		values: Vec<String>,
	) -> Option<Command> {
		if !self.interactions_allowed()
			|| self.interactions.busy()
			|| self.interactions.modal.is_some()
		{
			return None;
		}
		let source = self
			.timeline
			.get(message)
			.or_else(|| self.interactions.ephemeral.iter().find(|m| m.id == message))?;
		if source.forwarded || Some(source.channel) != self.selected {
			return None;
		}
		let c = source.components.iter().find_map(|c| c.find(custom_id))?;
		if c.disabled || custom_id.is_empty() || custom_id.chars().count() > 100 {
			return None;
		}
		if !match c.kind {
			2 => matches!(c.style, Some(1..=4)) && values.is_empty(),
			3 | 5..=8 => valid_values(c, &values, c.min_values.unwrap_or(1) > 0),
			_ => false,
		} {
			return None;
		}
		let application = source.application_id.or_else(|| {
			(!source.author.webhook
				&& matches!(
					source.author.kind,
					model::AccountKind::Bot | model::AccountKind::App
				))
			.then_some(source.author.id)
		})?;
		let flags = source.flags;
		let data = Data::Component {
			custom_id: custom_id.to_owned(),
			component_type: c.kind,
			values,
		};
		self.begin_interaction(application, Some(message), flags, data)
	}
	pub(crate) fn begin_interaction(
		&mut self,
		application_id: Id,
		message_id: Option<Id>,
		message_flags: u64,
		data: Data,
	) -> Option<Command> {
		let channel_id = self.selected?;
		let guild_id = self.channel(channel_id)?.guild;
		self.interactions.sequence = self.interactions.sequence.wrapping_add(1);
		let request = self.interactions.sequence;
		let nonce = crate::fingerprint::nonce(
			SystemTime::now()
				.duration_since(UNIX_EPOCH)
				.unwrap_or_default()
				.as_millis(),
			request,
		);
		self.interactions.pending = Some(Pending {
			nonce: nonce.clone(),
			channel: channel_id,
			application: application_id,
			message: message_id,
			deadline: Instant::now() + Duration::from_secs(30),
			modal: matches!(data, Data::Modal { .. }),
		});
		self.interactions.completed = None;
		self.interactions.error = None;
		Some(Command::Interaction(Request {
			request,
			nonce,
			application_id,
			channel_id,
			guild_id,
			message_id,
			message_flags,
			data,
		}))
	}
	pub fn submit_interaction_modal(&mut self, components: Vec<Component>) -> Option<Command> {
		if !self.interactions_allowed()
			|| self.interactions.busy()
			|| self.selected != self.interactions.modal_channel
		{
			return None;
		}
		let modal = self.interactions.modal.as_ref()?;
		if !model::valid_components(&components)
			|| !valid_modal_values(&modal.components, &components)
		{
			self.interactions.error = Some("Complete the required fields within their limits");
			return None;
		}
		let data = Data::Modal {
			id: modal.id,
			custom_id: modal.custom_id.clone(),
			components,
		};
		self.begin_interaction(modal.application_id, None, 0, data)
	}
	pub fn dismiss_interaction_modal(&mut self) {
		if !self.interactions.busy() {
			self.interactions.modal = None;
			self.interactions.modal_channel = None;
			self.interactions.error = None;
		}
	}
	pub fn dismiss_ephemeral(&mut self, id: Id) {
		self.interactions.ephemeral.retain(|m| m.id != id);
	}
	pub fn expire_interaction(&mut self, now: Instant) {
		if !self.interactions_allowed()
			&& (self.interactions.busy()
				|| self.interactions.modal.is_some()
				|| !self.interactions.ephemeral.is_empty())
		{
			self.interactions.reset();
		}
		if self
			.interactions
			.pending
			.as_ref()
			.is_some_and(|p| now >= p.deadline)
		{
			self.interactions.pending = None;
			self.interactions.error =
				Some("No response received; check the conversation before trying again");
		}
		if self
			.interactions
			.completed
			.as_ref()
			.is_some_and(|p| now >= p.deadline)
		{
			self.interactions.completed = None;
		}
	}
	pub fn apply_interaction(&mut self, event: Event) -> Result<(), &'static str> {
		if !self.interactions_allowed() {
			return Ok(());
		}
		match event {
			Event::Session(_) => {}
			Event::Ephemeral(message) => {
				if !message.ephemeral
					|| Some(message.channel) != self.selected
					|| !session_cache::Timeline::valid_message(&message)
					|| message.bytes() > 256 * 1024
				{
					return Ok(());
				}
				self.interactions.ephemeral.retain(|m| m.id != message.id);
				while self.interactions.ephemeral.len() >= 16
					|| self
						.interactions
						.ephemeral
						.iter()
						.map(Message::bytes)
						.sum::<usize>() + message.bytes()
						> 512 * 1024
				{
					self.interactions.ephemeral.remove(0);
				}
				self.interactions.ephemeral.push(*message);
			}
			Event::Modal { nonce, modal } => {
				let pending = self
					.interactions
					.pending
					.as_ref()
					.or(self.interactions.completed.as_ref());
				if !pending.is_some_and(|p| {
					p.nonce == nonce
						&& p.application == modal.application_id
						&& Some(p.channel) == self.selected
						&& Instant::now() < p.deadline
				}) || modal.id.0 == 0
					|| modal.title.chars().count() > 45
					|| modal.custom_id.is_empty()
					|| modal.custom_id.chars().count() > 100
					|| !model::valid_components(&modal.components)
				{
					return Ok(());
				}
				self.interactions.pending = None;
				self.interactions.completed = None;
				self.interactions.modal_channel = self.selected;
				self.interactions.modal = Some(*modal);
				self.interactions.error = None;
			}
			Event::Success { nonce } => {
				if self
					.interactions
					.pending
					.as_ref()
					.is_some_and(|p| p.nonce == nonce)
				{
					let p = self.interactions.pending.take().unwrap();
					if p.modal {
						self.interactions.modal = None;
						self.interactions.modal_channel = None;
					}
					self.interactions.completed = Some(p);
					self.interactions.error = None;
				}
			}
			Event::Failed { nonce } => self.interaction_failed(
				&nonce,
				"The application could not complete this interaction",
			),
			Event::Submitted {
				nonce,
				result: Err(failure),
			} => {
				self.interaction_failed(&nonce, failure.label());
				if failure.ends_session() {
					self.fail(failure);
				}
			}
			Event::Submitted {
				nonce,
				result: Ok(()),
			} => {
				if let Some(pending) = &mut self.interactions.pending
					&& pending.nonce == nonce
				{
					pending.deadline = Instant::now() + Duration::from_secs(30);
				}
			}
		}
		self.revision = self.revision.wrapping_add(1);
		Ok(())
	}
	fn interaction_failed(&mut self, nonce: &str, error: &'static str) {
		if self
			.interactions
			.pending
			.as_ref()
			.is_some_and(|p| p.nonce == nonce)
		{
			self.interactions.pending = None;
			self.interactions.error = Some(error);
		}
	}
}

impl State {
	pub(crate) fn handle_private_message_event(&mut self, event: &crate::Event) -> bool {
		match event {
			crate::Event::Message(message) if message.ephemeral => {
				let _ = self.apply_interaction(Event::Ephemeral(Box::new(message.clone())));
				true
			}
			crate::Event::Patch(patch) => {
				let Some(index) = self
					.interactions
					.ephemeral
					.iter()
					.position(|m| m.id == patch.id && m.channel == patch.channel)
				else {
					return matches!(patch.flags, model::Patch::Value(flags) if flags & 64 != 0);
				};
				let mut updated = self.interactions.ephemeral[index].clone();
				session_cache::apply_patch(&mut updated, patch);
				updated.ephemeral = true;
				updated.flags |= 64;
				let bytes = self
					.interactions
					.ephemeral
					.iter()
					.map(Message::bytes)
					.sum::<usize>() - self.interactions.ephemeral[index].bytes()
					+ updated.bytes();
				if updated.bytes() <= 256 * 1024
					&& bytes <= 512 * 1024
					&& session_cache::Timeline::valid_message(&updated)
				{
					self.interactions.ephemeral[index] = updated;
					self.revision = self.revision.wrapping_add(1);
				}
				true
			}
			crate::Event::Delete { channel, id } => {
				let before = self.interactions.ephemeral.len();
				self.interactions
					.ephemeral
					.retain(|m| m.channel != *channel || m.id != *id);
				before != self.interactions.ephemeral.len()
			}
			crate::Event::DeleteBulk { channel, ids } => {
				self.interactions
					.ephemeral
					.retain(|m| m.channel != *channel || !ids.contains(&m.id));
				false
			}
			_ => false,
		}
	}
}
