//! Recovery budgets that bound new sessions independently of normal socket commands.
use client_core::auth::Failure;
use futures_util::{Sink, SinkExt, Stream, StreamExt};
use std::collections::VecDeque;
use tokio::time::{Duration, Instant, sleep};
use tokio_tungstenite::tungstenite::{Error, Message as Frame};

const IDENTIFY_WINDOW: Duration = Duration::from_secs(60 * 60);
const MAX_IDENTIFIES: usize = 10;
const COMMAND_WINDOW: Duration = Duration::from_secs(60);
const MAX_COMMANDS: usize = 110;

/// Counts only fresh IDENTIFY handshakes, not successful session resumes.
#[derive(Default)]
pub struct IdentifyBudget {
	recent: VecDeque<Instant>,
}

impl IdentifyBudget {
	pub fn admit(&mut self, now: Instant) -> Result<(), Failure> {
		while self
			.recent
			.front()
			.is_some_and(|sent| now.duration_since(*sent) >= IDENTIFY_WINDOW)
		{
			self.recent.pop_front();
		}
		if self.recent.len() >= MAX_IDENTIFIES {
			return Err(Failure::CapacityAt(
				"Too many new Gateway sessions in one hour; wait before identifying again",
			));
		}
		self.recent.push_back(now);
		Ok(())
	}
}

struct RateWindow {
	sent: VecDeque<Instant>,
}

impl Default for RateWindow {
	fn default() -> Self {
		Self {
			sent: VecDeque::with_capacity(MAX_COMMANDS),
		}
	}
}

impl RateWindow {
	fn reserve(&mut self, now: Instant) -> Duration {
		while self
			.sent
			.front()
			.is_some_and(|sent| now.duration_since(*sent) >= COMMAND_WINDOW)
		{
			self.sent.pop_front();
		}
		if self.sent.len() >= MAX_COMMANDS {
			return self.sent.front().map_or(Duration::ZERO, |oldest| {
				(*oldest + COMMAND_WINDOW).saturating_duration_since(now)
			});
		}
		self.sent.push_back(now);
		Duration::ZERO
	}
}

/// Charges JSON gateway commands, but not heartbeat/control frames that keep the socket alive.
fn charge(frame: &Frame) -> bool {
	match frame {
		Frame::Text(text) => !text.split_once("\"op\":").is_some_and(|(_, operation)| {
			operation.as_bytes().first() == Some(&b'1')
				&& operation
					.as_bytes()
					.get(1)
					.is_none_or(|next| !next.is_ascii_digit())
		}),
		Frame::Ping(_) | Frame::Pong(_) | Frame::Close(_) => false,
		_ => true,
	}
}

/// One connection's command writer. The rate wait remains inside the gateway's select loop,
/// so heartbeat branches are still able to run while a normal command is queued.
pub struct CommandSocket<S> {
	socket: S,
	commands: RateWindow,
}

impl<S> CommandSocket<S>
where
	S: Sink<Frame, Error = Error> + Stream<Item = Result<Frame, Error>> + Unpin,
{
	pub fn new(socket: S) -> Self {
		Self {
			socket,
			commands: RateWindow::default(),
		}
	}

	pub async fn send(&mut self, frame: Frame) -> Result<(), Error> {
		if charge(&frame) {
			let wait = self.commands.reserve(Instant::now());
			if !wait.is_zero() {
				sleep(wait).await;
			}
		}
		self.socket.send(frame).await
	}

	pub async fn next(&mut self) -> Option<Result<Frame, Error>> {
		self.socket.next().await
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn fresh_sessions_stop_at_the_hourly_budget_and_age_out() {
		let start = Instant::now();
		let mut budget = IdentifyBudget::default();
		for index in 0..MAX_IDENTIFIES {
			assert!(
				budget
					.admit(start + Duration::from_secs(index as u64))
					.is_ok()
			);
		}
		assert!(matches!(
			budget.admit(start + Duration::from_secs(11)),
			Err(Failure::CapacityAt(_))
		));
		assert!(budget.admit(start + IDENTIFY_WINDOW).is_ok());
	}

	#[test]
	fn command_window_delays_instead_of_charging_heartbeats() {
		let start = Instant::now();
		let mut window = RateWindow::default();
		assert_eq!(window.reserve(start), Duration::ZERO);
		let heartbeat: Frame = serde_json::json!({"op":1,"d":null}).to_string().into();
		let command: Frame = serde_json::json!({"op":3,"d":{}}).to_string().into();
		assert!(!charge(&heartbeat), "{heartbeat}");
		assert!(charge(&command));
		assert!(!charge(&Frame::Ping(Vec::new().into())));
		assert!(!charge(&Frame::Pong(Vec::new().into())));
		assert!(!charge(&Frame::Close(None)));
		for index in 1..MAX_COMMANDS {
			assert_eq!(
				window.reserve(start + Duration::from_millis(index as u64)),
				Duration::ZERO
			);
		}
		assert_eq!(
			window.reserve(start + Duration::from_secs(1)),
			COMMAND_WINDOW - Duration::from_secs(1)
		);
		assert_eq!(window.reserve(start + COMMAND_WINDOW), Duration::ZERO);
	}
}
